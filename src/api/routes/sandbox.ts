import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import { z } from 'zod';
import { createSandboxForGroup } from '../../lib/sandbox';
import { requestContext } from '../../lib/request-context';

const createSandboxSchema = z.object({
  appId: z.string().optional(),
  ttlHours: z.number().int().positive().optional()
});

export default async function sandboxRoutes(fastify: FastifyInstance) {
  fastify.post('/groups', async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId) {
      return reply.code(401).send({ error: 'unauthorized' });
    }

    const parsed = createSandboxSchema.safeParse(req.body);
    if (!parsed.success) {
      return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    }

    const { appId, ttlHours } = parsed.data;

    const { sandboxGroup, session } = await createSandboxForGroup({
      sourceGroupId: ctx.groupId,
      appId,
      ttlHours
    });

    reply.send({ sandboxGroupId: sandboxGroup.id, sessionId: session.id });
  });
}
