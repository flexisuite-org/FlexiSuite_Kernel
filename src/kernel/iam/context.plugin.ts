import { FastifyInstance, FastifyRequest } from 'fastify';
import { setRlsContext } from '../../lib/db';
import { setRequestContext } from '../../lib/request-context';

declare module 'fastify' {
  interface FastifyRequest {
    user?: { id: string; groupId: string | null; roles?: string[] };
  }
}

export async function contextPlugin(fastify: FastifyInstance) {
  fastify.addHook('onRequest', async (req: FastifyRequest) => {
    const groupId = (req.user as any)?.groupId ?? null;
    const userId = (req.user as any)?.id ?? null;
    const mode = (req.headers['x-flexi-mode'] as string) === 'draft' ? 'draft' : 'stable';
    setRequestContext({ groupId, userId, mode });
    await setRlsContext(groupId, userId, mode);
  });
}
