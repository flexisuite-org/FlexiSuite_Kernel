"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.truncateAll = truncateAll;
exports.createTenantSeed = createTenantSeed;
exports.createPolicy = createPolicy;
exports.createPackage = createPackage;
const db_1 = require("../../src/lib/db");
const request_context_1 = require("../../src/lib/request-context");
const integrity_1 = require("../../src/lib/integrity");
async function truncateAll() {
    await db_1.prisma.$executeRawUnsafe(`
    TRUNCATE "PlaygroundLog",
            "ComponentInstall",
            "ComponentDependency",
            "ComponentPackage",
            "ComponentPolicy",
            "EntityHistory",
            "EntityRecord",
            "EntityDefinition",
            "AppInstall",
            "App",
            "RolePermission",
            "Permission",
            "Role",
            "GroupMember",
            "RefreshToken",
            "AuditLog",
            "Group",
            "User"
    RESTART IDENTITY CASCADE;
  `);
}
async function createTenantSeed(suffix) {
    const group = await db_1.prisma.group.create({ data: { name: `G-${suffix}`, type: 'ORG' } });
    const user = await db_1.prisma.user.create({ data: { email: `user+${suffix}@example.com`, passwordHash: 'x' } });
    return { groupId: group.id, userId: user.id };
}
async function createPolicy(name = 'default-policy') {
    const existing = await db_1.prisma.componentPolicy.findFirst({ where: { name } });
    if (existing)
        return existing.id;
    return (await db_1.prisma.componentPolicy.create({ data: { name } })).id;
}
async function createPackage(opts) {
    const manifest = {
        name: opts.name,
        version: opts.version,
        engine: '1.0.0',
        capabilities: opts.capabilities ?? ['echo']
    };
    const integrity = (0, integrity_1.hashJson)(manifest);
    const policyId = await createPolicy();
    (0, request_context_1.setRequestContext)({ groupId: opts.groupId, userId: opts.userId });
    return db_1.prisma.componentPackage.create({
        data: {
            name: manifest.name,
            version: manifest.version,
            status: opts.status ?? 'APPROVED',
            integrityHash: integrity,
            manifest,
            policyId,
            ownerGroupId: opts.groupId,
            createdById: opts.userId
        }
    });
}
//# sourceMappingURL=seed.js.map