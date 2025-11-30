"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.default = registryRoutes;
const db_1 = require("../../lib/db");
const request_context_1 = require("../../lib/request-context");
const integrity_1 = require("../../lib/integrity");
const signature_1 = require("../../lib/signature");
const config_1 = require("../../config");
const storage_1 = require("../../kernel/components/storage");
async function registryRoutes(fastify) {
    // list packages for current group
    fastify.get('/packages', async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        const user = req.user;
        const groupId = user?.groupId ?? ctx?.groupId;
        if (!groupId)
            return reply.code(400).send({ error: 'groupId required' });
        const packages = await db_1.prisma.componentPackage.findMany({
            where: { ownerGroupId: groupId },
            include: { dependencies: true }
        });
        reply.send(packages);
    });
    // register draft package
    fastify.post('/packages', { config: { rateLimit: { max: 20, timeWindow: '1 minute' } } }, async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        const user = req.user;
        const groupId = user?.groupId ?? ctx?.groupId;
        const userId = user?.id ?? ctx?.userId;
        if (!groupId || !userId)
            return reply.code(401).send({ error: 'unauthorized' });
        const body = req.body;
        const manifestBase = body.manifest;
        const integrity = (0, integrity_1.hashJson)(manifestBase);
        const bundleIntegrity = body.bundleIntegrity;
        const signingPayload = JSON.stringify({ manifestIntegrity: integrity, bundleIntegrity });
        const signature = config_1.config.SIGNING_SECRET ? (0, signature_1.signHmac)(signingPayload, config_1.config.SIGNING_SECRET) : undefined;
        const manifestToStore = manifestBase;
        const result = await db_1.prisma.$transaction(async (tx) => {
            const pkg = await tx.componentPackage.create({
                data: {
                    name: body.name,
                    version: body.version,
                    integrityHash: integrity,
                    bundleIntegrity,
                    bundleSignature: signature,
                    manifest: manifestToStore,
                    policyId: body.policyId,
                    ownerGroupId: groupId,
                    createdById: userId
                }
            });
            if (body.dependencies?.length) {
                await tx.componentDependency.createMany({
                    data: body.dependencies.map((d) => ({
                        packageId: pkg.id,
                        depName: d.name,
                        depVersion: d.version,
                        integrity: d.integrity,
                        kind: d.kind ?? 'RUNTIME'
                    }))
                });
            }
            return pkg;
        });
        reply.code(201).send(result);
    });
    // update bundle integrity/signature after upload
    fastify.post('/packages/:id/bundle', async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        const user = req.user;
        const groupId = user?.groupId ?? ctx?.groupId;
        if (!groupId)
            return reply.code(401).send({ error: 'unauthorized' });
        const { id } = req.params;
        const { bundleIntegrity } = req.body;
        if (!bundleIntegrity)
            return reply.code(400).send({ error: 'bundleIntegrity_required' });
        const signingPayload = JSON.stringify({ manifestIntegrity: undefined, bundleIntegrity });
        const signature = config_1.config.SIGNING_SECRET ? (0, signature_1.signHmac)(signingPayload, config_1.config.SIGNING_SECRET) : undefined;
        const updated = await db_1.prisma.componentPackage.updateMany({
            where: { id, ownerGroupId: groupId },
            data: { bundleIntegrity, bundleSignature: signature }
        });
        if (updated.count === 0)
            return reply.code(404).send({ error: 'not found' });
        reply.send({ id, bundleIntegrity, bundleSignature: signature });
    });
    fastify.post('/packages/:id/approve', async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        const user = req.user;
        const groupId = user?.groupId ?? ctx?.groupId;
        if (!groupId)
            return reply.code(401).send({ error: 'unauthorized' });
        const { id } = req.params;
        const updated = await db_1.prisma.componentPackage.updateMany({
            where: { id, ownerGroupId: groupId },
            data: { status: 'APPROVED', approvedAt: new Date() }
        });
        if (updated.count === 0)
            return reply.code(404).send({ error: 'not found' });
        reply.send({ id, status: 'APPROVED' });
    });
    fastify.post('/packages/:id/revoke', async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        const user = req.user;
        const groupId = user?.groupId ?? ctx?.groupId;
        if (!groupId)
            return reply.code(401).send({ error: 'unauthorized' });
        const { id } = req.params;
        const updated = await db_1.prisma.componentPackage.updateMany({
            where: { id, ownerGroupId: groupId },
            data: { status: 'REVOKED' }
        });
        if (updated.count === 0)
            return reply.code(404).send({ error: 'not found' });
        reply.send({ id, status: 'REVOKED' });
    });
    // upload bundle as base64, store integrity/signature and local file
    fastify.post('/packages/:id/bundleUpload', async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        const user = req.user;
        const groupId = user?.groupId ?? ctx?.groupId;
        if (!groupId)
            return reply.code(401).send({ error: 'unauthorized' });
        const { id } = req.params;
        const body = req.body;
        if (!body?.data || !body?.integrity)
            return reply.code(400).send({ error: 'invalid_input' });
        const buffer = Buffer.from(body.data, 'base64');
        const actual = (0, integrity_1.sha256Hex)(buffer);
        if (actual !== body.integrity)
            return reply.code(422).send({ error: 'integrity_mismatch', expected: body.integrity, got: actual });
        const signingPayload = JSON.stringify({ manifestIntegrity: undefined, bundleIntegrity: body.integrity });
        const signature = config_1.config.SIGNING_SECRET ? (0, signature_1.signHmac)(signingPayload, config_1.config.SIGNING_SECRET) : undefined;
        const updated = await db_1.prisma.componentPackage.updateMany({ where: { id, ownerGroupId: groupId }, data: { bundleIntegrity: body.integrity, bundleSignature: signature } });
        if (updated.count === 0)
            return reply.code(404).send({ error: 'not found' });
        await storage_1.bundleStorage.save(id, buffer);
        reply.code(201).send({ id, bundleIntegrity: body.integrity, bundleSignature: signature });
    });
}
//# sourceMappingURL=registry.js.map