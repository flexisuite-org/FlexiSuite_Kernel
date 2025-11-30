"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const supertest_1 = __importDefault(require("supertest"));
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const path_1 = __importDefault(require("path"));
const os_1 = __importDefault(require("os"));
const promises_1 = __importDefault(require("fs/promises"));
const util_1 = require("util");
const child_process_1 = require("child_process");
const server_1 = require("../src/api/server");
const db_1 = require("../src/lib/db");
const seed_1 = require("./helpers/seed");
const config_1 = require("../src/config");
const execAsync = (0, util_1.promisify)(child_process_1.exec);
function token(userId, groupId) {
    return jsonwebtoken_1.default.sign({ userId, groupId, roles: [] }, config_1.config.JWT_SECRET);
}
describe('github build workflow', () => {
    const app = (0, server_1.buildServer)();
    let groupId;
    let userId;
    let policyId;
    beforeEach(async () => {
        jest.setTimeout(30000);
        await app.ready();
        await (0, seed_1.truncateAll)();
        const seed = await (0, seed_1.createTenantSeed)(`gh-${Date.now()}`);
        groupId = seed.groupId;
        userId = seed.userId;
        policyId = await (0, seed_1.createPolicy)(`gh-pol-${Date.now()}`);
    });
    afterAll(async () => {
        await db_1.prisma.$disconnect();
        await app.close();
        const { closeRedis } = await Promise.resolve().then(() => __importStar(require('../src/lib/redis')));
        await closeRedis();
    });
    it('queues build job and produces bundle upload', async () => {
        const repoDir = await promises_1.default.mkdtemp(path_1.default.join(os_1.default.tmpdir(), 'gh-build-repo-'));
        const pkgName = `@demo/github-${Date.now()}`;
        await execAsync('git init -b main', { cwd: repoDir });
        await execAsync('git config user.email "tester@example.com"', { cwd: repoDir });
        await execAsync('git config user.name "Tester"', { cwd: repoDir });
        await promises_1.default.writeFile(path_1.default.join(repoDir, 'README.md'), '# demo');
        await execAsync('git add README.md', { cwd: repoDir });
        await execAsync('git commit -m "init"', { cwd: repoDir });
        const buildCommand = 'mkdir -p dist && echo "ok" > dist/out.txt';
        const enqueue = await (0, supertest_1.default)(app.server)
            .post('/integrations/github/build')
            .set('authorization', 'Bearer ' + token(userId, groupId))
            .send({
            repo: repoDir,
            branch: 'main',
            buildCommand,
            artifactPath: 'dist',
            packageName: pkgName,
            version: '1.0.0',
            policyId,
            approve: true
        });
        expect(enqueue.status).toBe(202);
        const jobId = enqueue.body.jobId;
        expect(jobId).toBeDefined();
        let status;
        for (let i = 0; i < 25; i++) {
            const res = await (0, supertest_1.default)(app.server)
                .get('/integrations/github/status')
                .set('authorization', 'Bearer ' + token(userId, groupId))
                .query({ jobId });
            if (res.status === 200) {
                status = res.body;
                if (status.status === 'done' || status.status === 'failed')
                    break;
            }
            await new Promise((r) => setTimeout(r, 300));
        }
        expect(status?.status).toBe('done');
        expect(status?.packageId).toBeTruthy();
        const pkg = await db_1.prisma.componentPackage.findFirst({
        where: { id: status.packageId }
      });
        expect(pkg?.bundleIntegrity).toBeTruthy();
        const bundlePath = path_1.default.join(config_1.config.bundleStorage.localDir, `${status.packageId}.bin`);
        const stat = await promises_1.default.stat(bundlePath);
        expect(stat.isFile()).toBe(true);
        await promises_1.default.rm(repoDir, { recursive: true, force: true });
    });
});
//# sourceMappingURL=github.build.spec.js.map
