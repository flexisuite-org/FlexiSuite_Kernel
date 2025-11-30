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
process.env.SIGNING_SECRET = process.env.SIGNING_SECRET || 'testsecret';
const supertest_1 = __importDefault(require("supertest"));
const server_1 = require("../src/api/server");
const db_1 = require("../src/lib/db");
const integrity_1 = require("../src/lib/integrity");
const config_1 = require("../src/config");
const request_context_1 = require("../src/lib/request-context");
const seed_1 = require("./helpers/seed");
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
function token(userId, groupId) {
    return jsonwebtoken_1.default.sign({ userId, groupId, roles: [] }, config_1.config.JWT_SECRET);
}
describe('bundle upload', () => {
    const app = (0, server_1.buildServer)();
    let groupId;
    let userId;
    let pkgId;
    let policyId;
    beforeEach(async () => {
        await app.ready();
        await (0, seed_1.truncateAll)();
        const seed = await (0, seed_1.createTenantSeed)(`bundle-${Date.now()}`);
        groupId = seed.groupId;
        userId = seed.userId;
        policyId = await (0, seed_1.createPolicy)(`pb-${Date.now()}`);
        const manifest = { name: `@gup/pkg-${Date.now()}`, version: '1.0.0', engine: '1.0.0', capabilities: [] };
        const integrity = (0, integrity_1.hashJson)(manifest);
        (0, request_context_1.setRequestContext)({ groupId, userId });
        pkgId = (await db_1.prisma.componentPackage.create({ data: { name: manifest.name, version: manifest.version, status: 'DRAFT', integrityHash: integrity, manifest, policyId, ownerGroupId: groupId, createdById: userId } })).id;
    });
    afterAll(async () => {
        await db_1.prisma.$disconnect();
        await app.close();
        const { closeRedis } = await Promise.resolve().then(() => __importStar(require('../src/lib/redis')));
        await closeRedis();
    });
    it('uploads bundle, verifies integrity, sets signature', async () => {
        const buf = Buffer.from('hello bundle');
        const integrity = (0, integrity_1.sha256Hex)(buf);
        const res = await (0, supertest_1.default)(app.server)
            .post(`/registry/packages/${pkgId}/bundleUpload`)
            .set('authorization', 'Bearer ' + token(userId, groupId))
            .send({ data: buf.toString('base64'), integrity });
        expect(res.status).toBe(201);
        expect(res.body.bundleIntegrity).toBe(integrity);
        expect(res.body.bundleSignature).toBeTruthy();
    });
});
//# sourceMappingURL=bundle.upload.spec.js.map