import { FastifyInstance } from 'fastify';
import { authService } from '../../kernel/iam/auth.service';
import { z } from 'zod';

export default async function authRoutes(fastify: FastifyInstance) {
  const signupSchema = z.object({ email: z.string().email(), password: z.string().min(8) });
  const loginSchema = signupSchema;
  const refreshSchema = z.object({ userId: z.string(), refreshToken: z.string(), familyId: z.string().optional() });

  fastify.post('/signup', { config: { rateLimit: { max: 10, timeWindow: '1 minute' } } }, async (req, reply) => {
    const parsed = signupSchema.safeParse(req.body);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const { email, password } = parsed.data;
    const tokens = await authService.signup(email, password);
    reply.send(tokens);
  });

  fastify.post('/login', { config: { rateLimit: { max: 20, timeWindow: '1 minute' } } }, async (req, reply) => {
    const parsed = loginSchema.safeParse(req.body);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const { email, password } = parsed.data;
    const tokens = await authService.login(email, password);
    reply.send(tokens);
  });

  fastify.post('/refresh', { config: { rateLimit: { max: 30, timeWindow: '1 minute' } } }, async (req, reply) => {
    const parsed = refreshSchema.safeParse(req.body);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const { userId, refreshToken, familyId } = parsed.data;
    const tokens = await authService.refresh(userId, refreshToken, familyId);
    reply.send(tokens);
  });
}
