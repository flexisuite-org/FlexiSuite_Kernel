import { FastifyInstance } from 'fastify';
import { authService } from '../../kernel/iam/auth.service';

export default async function authRoutes(fastify: FastifyInstance) {
  fastify.post('/signup', async (req, reply) => {
    const { email, password } = req.body as any;
    const tokens = await authService.signup(email, password);
    reply.send(tokens);
  });

  fastify.post('/login', async (req, reply) => {
    const { email, password } = req.body as any;
    const tokens = await authService.login(email, password);
    reply.send(tokens);
  });

  fastify.post('/refresh', async (req, reply) => {
    const { userId, refreshToken, familyId } = req.body as any;
    const tokens = await authService.refresh(userId, refreshToken, familyId);
    reply.send(tokens);
  });
}
