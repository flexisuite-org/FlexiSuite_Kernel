import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import jwt from 'jsonwebtoken';
import { config } from '../../config';

interface JwtPayload {
  userId: string;
  groupId?: string | null;
  roles?: string[];
}

export async function authHook(fastify: FastifyInstance) {
  fastify.addHook('onRequest', async (req: FastifyRequest, reply: FastifyReply) => {
    const auth = req.headers.authorization;
    if (!auth || !auth.startsWith('Bearer ')) return;
    const token = auth.slice('Bearer '.length);
    try {
      const payload = jwt.verify(token, config.JWT_SECRET) as JwtPayload;
      req.user = {
        id: payload.userId,
        groupId: payload.groupId ?? null,
        roles: payload.roles ?? []
      };
    } catch (err) {
      // invalid token -> clear user and proceed; protected routes should still reject
      req.user = undefined;
      reply.header('WWW-Authenticate', 'Bearer error="invalid_token"');
    }
  });
}
