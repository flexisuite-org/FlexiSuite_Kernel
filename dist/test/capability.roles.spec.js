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
process.env.CAPABILITY_ROLE_ALLOWLIST = process.env.CAPABILITY_ROLE_ALLOWLIST || JSON.stringify({ echo: ['admin'] });
const supertest_1 = __importDefault(require("supertest"));
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const db_1 = require("../src/lib/db");
const integrity_1 = require("../src/lib/integrity");
const request_context_1 = require("../src/lib/request-context");
const seed_1 = require("./helpers/seed");
function token(userId, groupId, roles) {
    const { config } = require('../src/config');
    return jsonwebtoken_1.default.sign({ userId, groupId, roles }, config.JWT_SECRET);
}
describe('capability role allowlist', () => {
    let buildServer;
    beforeAll(() => {
        jest.resetModules();
        buildServer = require('../src/api/server').buildServer;
    });
    afterAll(async () => {
        jest.resetModules();
        await db_1.prisma.$disconnect();
        const { closeRedis } = await Promise.resolve().then(() => __importStar(require('../src/lib/redis')));
        await closeRedis();
    });
    it('blocks capability when required role missing, allows when present', async () => {
        const app = buildServer();
        await app.ready();
        await (0, seed_1.truncateAll)();
        const seed = await (0, seed_1.createTenantSeed)(`role-${Date.now()}`);
        const groupId = seed.groupId;
        const userId = seed.userId;
        const policyId = await (0, seed_1.createPolicy)(`pol-${Date.now()}`);
        const manifest = { name: `@role/echo-${Date.now()}`, version: '1.0.0', engine: '1.0.0', capabilities: ['echo'] };
        const integrity = (0, integrity_1.hashJson)(manifest);
        (0, request_context_1.setRequestContext)({ groupId, userId });
        const pkg = await db_1.prisma.componentPackage.create({
            data: { name: manifest.name, version: manifest.version, status: 'APPROVED', integrityHash: integrity, manifest, policyId, ownerGroupId: groupId }
        });
        const install = await db_1.prisma.componentInstall.create({ data: { packageId: pkg.id, groupId, lockData: { integrity } } });
        const resNoRole = await (0, supertest_1.default)(app.server)
            .post(`/components/${install.id}/run`)
            .set('authorization', 'Bearer ' + token(userId, groupId, []))
            .send({ payload: {} });
        expect(resNoRole.status).toBe(200);
        expect(resNoRole.body.results['echo'].error).toBe('forbidden');
        const resWithRole = await (0, supertest_1.default)(app.server)
            .post(`/components/${install.id}/run`)
            .set('authorization', 'Bearer ' + token(userId, groupId, ['admin']))
            .send({ payload: {} });
        expect(resWithRole.status).toBe(200);
        expect(resWithRole.body.results['echo'].error).toBeUndefined();
        await app.close();
    });
});
//# sourceMappingURL=capability.roles.spec.js.map