import { FastifyInstance } from 'fastify';
import crypto from 'crypto';
import { authService } from '../../kernel/iam/auth.service';
import { prisma } from '../../lib/db';
import { z } from 'zod';

export default async function authRoutes(fastify: FastifyInstance) {
  const baseCredentials = z.object({ email: z.string().email(), password: z.string().min(8) });
  const signupSchema = baseCredentials.extend({ accountInviteCode: z.string().min(1).trim() });
  const loginSchema = baseCredentials;
  const refreshSchema = z.object({ userId: z.string(), refreshToken: z.string(), familyId: z.string().optional() });
  const accountInviteCreateSchema = z.object({
    email: z.string().email(),
    expiresAt: z.string().datetime().optional(),
    initialGroupId: z.string().optional()
  });

  fastify.post('/account-invites', async (req, reply) => {
    const user = (req as any).user;
    if (!user?.id) return reply.code(401).send({ error: 'unauthorized' });

    const parsed = accountInviteCreateSchema.safeParse(req.body);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });

    const normalizedEmail = parsed.data.email.trim().toLowerCase();
    let expiresAt: Date | null = null;
    if (parsed.data.expiresAt) {
      const parsedDate = new Date(parsed.data.expiresAt);
      if (Number.isNaN(parsedDate.getTime())) {
        return reply.code(400).send({ error: 'invalid_expires_at' });
      }
      expiresAt = parsedDate;
    }

    const code = crypto.randomBytes(10).toString('hex');
    const invite = await prisma.accountInvite.create({
      data: {
        email: normalizedEmail,
        code,
        expiresAt,
        initialGroupId: parsed.data.initialGroupId,
        createdBy: user.id
      }
    });

    reply.code(201).send({ code: invite.code, expiresAt: invite.expiresAt });
  });

  fastify.get('/account-invites/:code', async (req, reply) => {
    const { code } = req.params as { code: string };
    const invite = await prisma.accountInvite.findUnique({ where: { code } });
    if (!invite) return reply.code(404).send({ error: 'not_found' });
    if (invite.usedAt) return reply.code(409).send({ error: 'already_used' });
    if (invite.expiresAt && invite.expiresAt.getTime() <= Date.now()) {
      return reply.code(410).send({ error: 'expired' });
    }

    reply.send({
      email: invite.email,
      initialGroupId: invite.initialGroupId,
      expiresAt: invite.expiresAt
    });
  });

  fastify.post('/signup', { config: { rateLimit: { max: 10, timeWindow: '1 minute' } } }, async (req, reply) => {
    const parsed = signupSchema.safeParse(req.body);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });

    const email = parsed.data.email.trim().toLowerCase();
    const password = parsed.data.password;
    const invite = await prisma.accountInvite.findUnique({ where: { code: parsed.data.accountInviteCode } });
    if (!invite) return reply.code(404).send({ error: 'invite_not_found' });
    if (invite.usedAt) return reply.code(409).send({ error: 'invite_used' });
    if (invite.expiresAt && invite.expiresAt.getTime() <= Date.now()) {
      return reply.code(410).send({ error: 'invite_expired' });
    }
    if (invite.email !== email) {
      return reply.code(400).send({ error: 'email_mismatch' });
    }

    const tokens = await authService.signup(email, password);
    await prisma.accountInvite.update({ where: { id: invite.id }, data: { usedAt: new Date() } });
    // TODO: future support for auto-accepting groupInviteCode list.
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

  fastify.get('/me', async (req, reply) => {
    const user = (req as any).user;
    if (!user) return reply.code(401).send({ error: 'unauthorized' });

    const record = await prisma.user.findUnique({ where: { id: user.id } });
    if (!record) return reply.code(401).send({ error: 'unauthorized' });

    const memberships = await prisma.groupMember.findMany({
      where: { userId: user.id },
      include: {
        group: { select: { id: true, name: true, type: true } },
        roles: { select: { name: true } }
      }
    });

    reply.send({
      userId: user.id,
      email: record.email,
      roles: user.roles ?? [],
      memberships: memberships.map((membership: any) => ({
        groupId: membership.groupId,
        name: membership.group?.name ?? null,
        type: membership.group?.type ?? null,
        membershipRoles: membership.roles.map((role: any) => role.name)
      }))
    });
  });
}
