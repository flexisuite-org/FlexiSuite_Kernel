import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import { z } from 'zod';
import {
  CloneEntitySpec,
  CloneEntitiesSummary,
  cloneEntitiesForSandboxSession,
  ensureEntitiesForSandboxSession
} from '../../lib/sandbox';
import { prisma } from '../../lib/db';
import { requestContext } from '../../lib/request-context';

const createSandboxSchema = z.object({
  appId: z.string().optional(),
  ttlHours: z.number().int().positive().optional()
});

const cloneSpecSchema = z.object({
  model: z.enum(['EntityRecord', 'AppInstall']),
  ids: z.array(z.string()).optional(),
  whereJson: z.unknown().optional()
});

const cloneEntitiesBodySchema = z.object({
  specs: z.array(cloneSpecSchema)
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

  const handleSandboxError = (reply: FastifyReply, error: unknown) => {
    if (error instanceof Error) {
      switch (error.message) {
        case 'sandbox_session_not_found':
          return reply.code(404).send({ error: error.message });
        case 'sandbox_session_expired':
          return reply.code(410).send({ error: error.message });
        case 'no_specs':
          return reply.code(400).send({ error: error.message });
      }
    }
    return reply.code(500).send({ error: 'internal_error', message: error instanceof Error ? error.message : 'unknown_error' });
  };

  const createEntityHandler =
    (executor: (sessionId: string, specs: CloneEntitySpec[]) => Promise<CloneEntitiesSummary>) =>
    async (
      req: FastifyRequest<{ Params: { sessionId: string } }>,
      reply: FastifyReply
    ) => {
      const ctx = requestContext.getStore();
      if (!ctx?.groupId) {
        return reply.code(401).send({ error: 'unauthorized' });
      }

      const { sessionId } = req.params;

      const sessionCheck = await prisma.sandboxSession.findUnique({
        where: { id: sessionId },
        select: { sourceGroupId: true }
      });
      if (!sessionCheck) {
        return reply.code(404).send({ error: 'sandbox_session_not_found' });
      }

      if (sessionCheck.sourceGroupId !== ctx.groupId) {
        return reply.code(403).send({ error: 'sandbox_session_forbidden' });
      }

      const parsed = cloneEntitiesBodySchema.safeParse(req.body);
      if (!parsed.success) {
        return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
      }

      try {
        const result = await executor(sessionId, parsed.data.specs);
        return reply.send(result);
      } catch (error) {
        return handleSandboxError(reply, error);
      }
    };

  fastify.post('/sessions/:sessionId/clone-entities', createEntityHandler(cloneEntitiesForSandboxSession));
  fastify.post(
    '/sessions/:sessionId/ensure-entities',
    createEntityHandler((sessionId, specs) => ensureEntitiesForSandboxSession({ sessionId, specs }))
  );
}
