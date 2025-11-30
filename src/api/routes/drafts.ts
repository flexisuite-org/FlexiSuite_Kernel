import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import { sandbox } from '../../kernel/runtime/sandbox';
import { requestContext } from '../../lib/request-context';
import { prisma } from '../../lib/db';
import { saveDraftResult } from '../../lib/playground-db';
import { z } from 'zod';
import { closeRedis } from '../../lib/redis';

const draftRunSchema = z.object({
  script: z.string().min(1),
  payload: z.any().optional()
});

export default async function draftsRoutes(fastify: FastifyInstance) {
  // Draft sandbox execution (playground only, no persistent writes)
  fastify.post('/sandbox/drafts/run', async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId) return reply.code(401).send({ error: 'unauthorized' });

    const parsed = draftRunSchema.safeParse(req.body);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const body = parsed.data;

    // mark context as draft to block DB writes except playground log
    requestContext.enterWith({ ...ctx, mode: 'draft' });

    try {
      const result = await sandbox.run(body.script, {
        kernel: { groupId: ctx.groupId, userId: ctx.userId, payload: body.payload, channel: 'draft' }
      });

      await saveDraftResult(ctx.groupId, ctx.userId ?? null, { result });
      await prisma.auditLog.create({
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
    } catch (err: any) {
      const code = err?.name === 'OperationTimeoutError' ? 504 : 500;
      const message = err?.message || 'sandbox_error';
      await prisma.auditLog.create({
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
    await closeRedis().catch(() => {});
  });
}
