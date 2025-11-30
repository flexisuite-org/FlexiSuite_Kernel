"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.default = componentsRoutes;
const db_1 = require("../../lib/db");
const request_context_1 = require("../../lib/request-context");
const integrity_1 = require("../../lib/integrity");
const signature_1 = require("../../lib/signature");
const config_1 = require("../../config");
const capabilities_1 = require("../../kernel/components/capabilities");
const zod_1 = require("zod");
// Minimal run/bundle placeholders using install lock
async function componentsRoutes(fastify) {
    const runSchema = zod_1.z.object({ payload: zod_1.z.any().optional() });
    fastify.get('/components/:id/bundle', async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        const user = req.user;
        const groupId = user?.groupId ?? ctx?.groupId;
        if (!groupId)
            return reply.code(401).send({ error: 'unauthorized' });
        const { id } = req.params;
        const install = await db_1.prisma.componentInstall.findFirst({
            where: { id, groupId },
            include: { package: true }
        });
        if (!install)
            return reply.code(404).send({ error: 'not found' });
        reply.send({ manifest: install.package.manifest, integrity: install.package.integrityHash, lock: install.lockData });
    });
    fastify.post('/components/:id/run', async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        const user = req.user;
        const groupId = user?.groupId ?? ctx?.groupId;
        const userId = user?.id ?? ctx?.userId;
        if (!groupId)
            return reply.code(401).send({ error: 'unauthorized' });
        const ctxResolved = { groupId, userId, mode: ctx?.mode || 'stable' };
        if (!ctx || ctx.groupId !== groupId || ctx.userId !== userId) {
            (0, request_context_1.setRequestContext)({ groupId, userId, mode: ctx?.mode || 'stable' });
        }
        const parsedBody = runSchema.safeParse(req.body ?? {});
        if (!parsedBody.success)
            return reply.code(400).send({ error: 'invalid_input', details: parsedBody.error.flatten() });
        const inputPayload = parsedBody.data.payload;
        const { id } = req.params;
        const install = await db_1.prisma.componentInstall.findFirst({
            where: { id, groupId },
            include: { package: { include: { policy: true } } }
        });
        if (!install)
            return reply.code(404).send({ error: 'not found' });
        if (install.groupId !== groupId)
            return reply.code(404).send({ error: 'not_found_scope' });
        // integrity check: lock integrity vs stored hash (tamper detection)
        const lockIntegrity = install.lockData?.integrity;
        if (lockIntegrity && lockIntegrity !== install.package.integrityHash) {
            await db_1.prisma.auditLog.create({
                data: {
                    actorUserId: ctxResolved.userId,
                    groupId: ctxResolved.groupId,
                    resource: 'component.run',
                    action: 'integrity_mismatch',
                    metadata: { installId: install.id, expected: install.package.integrityHash, got: lockIntegrity },
                    success: false
                }
            });
            return reply.code(422).send({ error: 'integrity_mismatch' });
        }
        // verify manifest integrity & signature at runtime as well
        const manifestRaw = install.package.manifest;
        const manifestStr = (0, integrity_1.stableStringify)(manifestRaw);
        if (!(0, integrity_1.verifyIntegrity)(install.package.integrityHash, manifestRaw)) {
            await db_1.prisma.auditLog.create({
                data: {
                    actorUserId: ctxResolved.userId,
                    groupId: ctxResolved.groupId,
                    resource: 'component.run',
                    action: 'integrity_mismatch_manifest',
                    metadata: { installId: install.id },
                    success: false
                }
            });
            return reply.code(422).send({ error: 'integrity_mismatch_manifest' });
        }
        if (manifestRaw?.signature && config_1.config.SIGNING_SECRET) {
            const sig = manifestRaw.signature;
            if (!(0, signature_1.verifyHmac)(manifestStr, sig, config_1.config.SIGNING_SECRET)) {
                await db_1.prisma.auditLog.create({
                    data: {
                        actorUserId: ctxResolved.userId,
                        groupId: ctxResolved.groupId,
                        resource: 'component.run',
                        action: 'signature_mismatch',
                        metadata: { installId: install.id },
                        success: false
                    }
                });
                return reply.code(422).send({ error: 'signature_mismatch' });
            }
        }
        if (install.package.bundleIntegrity && config_1.config.SIGNING_SECRET) {
            const signingPayload = JSON.stringify({ manifestIntegrity: install.package.integrityHash, bundleIntegrity: install.package.bundleIntegrity });
            if (!install.package.bundleSignature || !(0, signature_1.verifyHmac)(signingPayload, install.package.bundleSignature, config_1.config.SIGNING_SECRET)) {
                await db_1.prisma.auditLog.create({
                    data: {
                        actorUserId: ctxResolved.userId,
                        groupId: ctxResolved.groupId,
                        resource: 'component.run',
                        action: 'bundle_signature_mismatch',
                        metadata: { installId: install.id },
                        success: false
                    }
                });
                return reply.code(422).send({ error: 'bundle_signature_mismatch' });
            }
        }
        // APIモード: capabilities に基づき限定的な処理のみ実行
        const manifest = install.package.manifest;
        const requested = manifest.allowedCapabilities ?? manifest.capabilities ?? [];
        const allowed = requested.filter((cap) => config_1.config.capabilityAllowlist.includes(cap));
        const denied = requested.filter((cap) => !config_1.config.capabilityAllowlist.includes(cap));
        const roles = (user?.roles ?? []);
        const payload = inputPayload;
        const results = {};
        if (allowed.length === 0) {
            for (const cap of denied) {
                results[cap] = { error: 'unsupported_capability' };
            }
        }
        for (const cap of requested) {
            if (!allowed.includes(cap))
                continue;
            const requiredRoles = config_1.config.capabilityRoleAllowlist[cap];
            if (requiredRoles && requiredRoles.length > 0) {
                const hasRole = roles.some((r) => requiredRoles.includes(r));
                if (!hasRole) {
                    results[cap] = { error: 'forbidden', reason: 'role_required' };
                    continue;
                }
            }
            const handler = capabilities_1.capabilityHandlers[cap];
            if (!handler) {
                results[cap] = { error: 'unsupported_capability' };
                continue;
            }
            try {
                results[cap] = await handler(payload);
            }
            catch (err) {
                results[cap] = { error: err?.message || 'capability_error' };
            }
        }
        await db_1.prisma.auditLog.create({
            data: {
                actorUserId: ctxResolved.userId,
                groupId: ctxResolved.groupId,
                resource: 'component.run',
                action: 'api',
                metadata: { installId: install.id, packageId: install.packageId, mode: 'API', capabilities: requested },
                success: true
            }
        });
        reply.send({ status: 'ok', mode: 'API', results, manifest: install.package.manifest, lock: install.lockData });
    });
}
//# sourceMappingURL=components.js.map