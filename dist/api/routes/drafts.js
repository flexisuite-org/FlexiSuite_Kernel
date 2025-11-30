"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.default = draftsRoutes;
const sandbox_1 = require("../../kernel/runtime/sandbox");
const request_context_1 = require("../../lib/request-context");
const db_1 = require("../../lib/db");
const playground_db_1 = require("../../lib/playground-db");
const zod_1 = require("zod");
const redis_1 = require("../../lib/redis");
const draftRunSchema = zod_1.z.object({
    script: zod_1.z.string().min(1),
    payload: zod_1.z.any().optional()
});
async function draftsRoutes(fastify) {
    // Draft sandbox execution (playground only, no persistent writes)
    fastify.post('/sandbox/drafts/run', async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        if (!ctx?.groupId)
            return reply.code(401).send({ error: 'unauthorized' });
        const parsed = draftRunSchema.safeParse(req.body);
        if (!parsed.success)
            return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
        const body = parsed.data;
        // mark context as draft to block DB writes except playground log
        request_context_1.requestContext.enterWith({ ...ctx, mode: 'draft' });
        try {
            const result = await sandbox_1.sandbox.run(body.script, {
                kernel: { groupId: ctx.groupId, userId: ctx.userId, payload: body.payload, channel: 'draft' }
            });
            await (0, playground_db_1.saveDraftResult)(ctx.groupId, ctx.userId ?? null, { result });
            await db_1.prisma.auditLog.create({
                data: {
                    actorUserId: ctx.userId,
                    groupId: ctx.groupId,
                    resource: 'sandbox.draft',
                    action: 'run',
                    metadata: { success: true },
                    success: true
                }
            });
            reply.send({ status: 'ok', result });
        }
        catch (err) {
            const code = err?.name === 'OperationTimeoutError' ? 504 : 500;
            const message = err?.message || 'sandbox_error';
            await db_1.prisma.auditLog.create({
                data: {
                    actorUserId: ctx.userId,
                    groupId: ctx.groupId,
                    resource: 'sandbox.draft',
                    action: 'run',
                    metadata: { success: false, error: message },
                    success: false
                }
            });
            reply.code(code).send({ error: 'sandbox_error', message });
        }
    });
    fastify.addHook('onClose', async () => {
        await (0, redis_1.closeRedis)().catch(() => { });
    });
}
//# sourceMappingURL=drafts.js.map