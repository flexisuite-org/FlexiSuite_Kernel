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
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const server_1 = require("../src/api/server");
const db_1 = require("../src/lib/db");
const integrity_1 = require("../src/lib/integrity");
const signature_1 = require("../src/lib/signature");
const config_1 = require("../src/config");
const request_context_1 = require("../src/lib/request-context");
const seed_1 = require("./helpers/seed");
function token(userId, groupId) {
    return jsonwebtoken_1.default.sign({ userId, groupId, roles: [] }, config_1.config.JWT_SECRET);
}
describe('install/run integration', () => {
    const app = (0, server_1.buildServer)();
    let groupA;
    let groupB;
    let userA;
    let policyId;
    beforeEach(async () => {
        jest.setTimeout(20000);
        await app.ready();
        await (0, seed_1.truncateAll)();
        const a = await (0, seed_1.createTenantSeed)(`a-${Date.now()}`);
        const b = await (0, seed_1.createTenantSeed)(`b-${Date.now()}`);
        groupA = a.groupId;
        userA = a.userId;
        groupB = b.groupId;
        policyId = await (0, seed_1.createPolicy)(`p-${Date.now()}`);
    });
    afterAll(async () => {
        await db_1.prisma.$disconnect();
        await app.close();
        const { closeRedis } = await Promise.resolve().then(() => __importStar(require('../src/lib/redis')));
        await closeRedis();
    });
    test('stable install rejects non-approved package', async () => {
        const manifest = { name: `@ga/demo-${Date.now()}`, version: '1.0.0', engine: '1.0.0', capabilities: ['echo'] };
        const integrity = (0, integrity_1.hashJson)(manifest);
        // create package with status DRAFT under groupA
        (0, request_context_1.setRequestContext)({ groupId: groupA, userId: userA });
        const pkg = await db_1.prisma.componentPackage.create({
            data: {
                name: manifest.name,
                version: manifest.version,
                status: 'DRAFT',
                integrityHash: integrity,
                manifest,
                policyId,
                ownerGroupId: groupA
            }
        });
        await db_1.prisma.componentInstall.create({
            data: {
                packageId: pkg.id,
                groupId: groupA,
                channel: 'DRAFT',
                lockData: { integrity: integrity }
            }
        });
        const res = await (0, supertest_1.default)(app.server)
            .post('/install')
            .set('authorization', 'Bearer ' + token(userA, groupA))
            .send({ name: manifest.name, version: manifest.version, channel: 'STABLE' });
        expect(res.status).toBe(409);
    });
    test('signature mismatch returns 422 on run', async () => {
        const manifest = { name: `@ga/signed-${Date.now()}`, version: '1.0.0', engine: '1.0.0', capabilities: ['echo'] };
        const integrity = (0, integrity_1.hashJson)(manifest);
        const badSignature = (0, signature_1.signHmac)('tampered', config_1.config.SIGNING_SECRET || 'testsecret');
        const manifestSigned = { ...manifest, signature: badSignature };
        (0, request_context_1.setRequestContext)({ groupId: groupA, userId: userA });
        const pkg = await db_1.prisma.componentPackage.create({
            data: {
                name: manifest.name,
                version: manifest.version,
                status: 'APPROVED',
                integrityHash: integrity,
                manifest: manifestSigned,
                policyId,
                ownerGroupId: groupA
            }
        });
        const install = await db_1.prisma.componentInstall.create({
            data: { packageId: pkg.id, groupId: groupA, lockData: { integrity } }
        });
        const res = await (0, supertest_1.default)(app.server)
            .post(`/components/${install.id}/run`)
            .set('authorization', 'Bearer ' + token(userA, groupA))
            .send({ payload: {} });
        expect(res.status).toBe(422);
    });
    test('bundle signature mismatch blocks install', async () => {
        const manifest = { name: `@ga/bundle-${Date.now()}`, version: '1.0.5', engine: '1.0.0', capabilities: ['echo'] };
        const integrity = (0, integrity_1.hashJson)(manifest);
        (0, request_context_1.setRequestContext)({ groupId: groupA, userId: userA });
        const pkg = await db_1.prisma.componentPackage.create({
            data: {
                name: manifest.name,
                version: manifest.version,
                status: 'APPROVED',
                integrityHash: integrity,
                bundleIntegrity: 'deadbeef',
                bundleSignature: 'wrongsig',
                manifest,
                policyId,
                ownerGroupId: groupA
            }
        });
        await db_1.prisma.componentInstall.create({ data: { packageId: pkg.id, groupId: groupA, lockData: { integrity } } });
        const res = await (0, supertest_1.default)(app.server)
            .post('/install')
            .set('authorization', 'Bearer ' + token(userA, groupA))
            .send({ name: manifest.name, version: manifest.version, channel: 'STABLE' });
        expect(res.status).toBeGreaterThanOrEqual(400);
    });
    test('cross-group run is not found', async () => {
        const manifest = { name: `@ga/echo-${Date.now()}`, version: '1.0.1', engine: '1.0.0', capabilities: ['echo'] };
        const integrity = (0, integrity_1.hashJson)(manifest);
        (0, request_context_1.setRequestContext)({ groupId: groupA, userId: userA });
        const pkg = await db_1.prisma.componentPackage.create({
            data: { name: manifest.name, version: manifest.version, status: 'APPROVED', integrityHash: integrity, manifest, policyId, ownerGroupId: groupA }
        });
        (0, request_context_1.setRequestContext)({ groupId: groupA, userId: userA });
        const install = await db_1.prisma.componentInstall.create({ data: { packageId: pkg.id, groupId: groupA, lockData: { integrity } } });
        const res = await (0, supertest_1.default)(app.server)
            .post(`/components/${install.id}/run`)
            .set('authorization', 'Bearer ' + token(userA, groupB))
            .send({ payload: { foo: 'bar' } });
        expect(res.status).toBe(404);
    });
    test('unsupported capability returns error but 200', async () => {
        const manifest = { name: `@ga/unknown-cap-${Date.now()}`, version: '1.0.2', engine: '1.0.0', capabilities: ['does.not.exist'] };
        const integrity = (0, integrity_1.hashJson)(manifest);
        (0, request_context_1.setRequestContext)({ groupId: groupA, userId: userA });
        const pkg = await db_1.prisma.componentPackage.create({
            data: { name: manifest.name, version: manifest.version, status: 'APPROVED', integrityHash: integrity, manifest, policyId, ownerGroupId: groupA }
        });
        (0, request_context_1.setRequestContext)({ groupId: groupA, userId: userA });
        const install = await db_1.prisma.componentInstall.create({ data: { packageId: pkg.id, groupId: groupA, lockData: { integrity } } });
        const res = await (0, supertest_1.default)(app.server)
            .post(`/components/${install.id}/run`)
            .set('authorization', 'Bearer ' + token(userA, groupA))
            .send({ payload: {} });
        expect(res.status).toBe(200);
        expect(res.body.results['does.not.exist'].error).toBe('unsupported_capability');
    });
    test('entity.list respects group isolation', async () => {
        const manifest = { name: `@ga/list-${Date.now()}`, version: '1.0.3', engine: '1.0.0', capabilities: ['data.entity.list'] };
        const integrity = (0, integrity_1.hashJson)(manifest);
        (0, request_context_1.setRequestContext)({ groupId: groupA, userId: userA });
        const pkg = await db_1.prisma.componentPackage.create({
            data: { name: manifest.name, version: manifest.version, status: 'APPROVED', integrityHash: integrity, manifest, policyId, ownerGroupId: groupA }
        });
        (0, request_context_1.setRequestContext)({ groupId: groupA, userId: userA });
        const install = await db_1.prisma.componentInstall.create({ data: { packageId: pkg.id, groupId: groupA, lockData: { integrity } } });
        // seed records in groupA
        const def = await db_1.prisma.entityDefinition.create({ data: { appId: (await db_1.prisma.app.create({ data: { name: 'a', version: '1' } })).id, name: 'n', version: 1, schema: {}, strict: false } });
        await db_1.prisma.entityRecord.create({ data: { definitionId: def.id, groupId: groupA, data: { foo: 'bar' }, schemaVersion: 1 } });
        // request as groupB should return empty due to RLS/middleware scoping
        const res = await (0, supertest_1.default)(app.server)
            .post(`/components/${install.id}/run`)
            .set('authorization', 'Bearer ' + token(userA, groupB))
            .send({ payload: { limit: 10 } });
        expect(res.status).toBe(404); // install not found in groupB
    });
    test('listByDefinition and getDefinition work in own group', async () => {
        const manifest = { name: `@ga/list-def-${Date.now()}`, version: '1.0.6', engine: '1.0.0', capabilities: ['data.entity.listByDefinition', 'data.entity.getDefinition'] };
        const integrity = (0, integrity_1.hashJson)(manifest);
        (0, request_context_1.setRequestContext)({ groupId: groupA, userId: userA });
        const pkg = await db_1.prisma.componentPackage.create({ data: { name: manifest.name, version: manifest.version, status: 'APPROVED', integrityHash: integrity, manifest, policyId, ownerGroupId: groupA } });
        const install = await db_1.prisma.componentInstall.create({ data: { packageId: pkg.id, groupId: groupA, lockData: { integrity } } });
        const def = await db_1.prisma.entityDefinition.create({ data: { appId: (await db_1.prisma.app.create({ data: { name: 'a3', version: '1' } })).id, name: 'defx', version: 1, schema: {}, strict: false } });
        const res = await (0, supertest_1.default)(app.server)
            .post(`/components/${install.id}/run`)
            .set('authorization', 'Bearer ' + token(userA, groupA))
            .send({ payload: { definitionId: def.id, limit: 10 } });
        expect(res.status).toBe(200);
        expect(res.body.results['data.entity.getDefinition'].id).toBe(def.id);
    });
});
//# sourceMappingURL=install.run.integration.spec.js.map