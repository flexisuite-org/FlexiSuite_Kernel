import { FastifyInstance, FastifyRequest } from 'fastify';
import { setRlsContext } from '../../lib/db';

declare module 'fastify' {
  interface FastifyRequest {
    user?: { id: string; groupId: string | null; roles?: string[] };
  }
}

export async function contextPlugin(fastify: FastifyInstance) {
  fastify.addHook('onRequest', async (req: FastifyRequest) => {
    const groupId = (req.user as any)?.groupId ?? null;
    const userId = (req.user as any)?.id ?? null;
    await setRlsContext(groupId, userId);
  });
}
