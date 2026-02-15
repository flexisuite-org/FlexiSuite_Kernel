import { FastifyInstance } from 'fastify';
import { prisma } from '../../lib/db';
import { z } from 'zod';
import crypto from 'crypto';
import { requestContext } from '../../lib/request-context';

export default async function invitesRoutes(fastify: FastifyInstance) {
  fastify.get('/pending', async (req, reply) => {
    const user = (req as any).user;
    if (!user?.id) return reply.code(401).send({ error: 'unauthorized' });

    const record = await prisma.user.findUnique({ where: { id: user.id } });
    if (!record || !record.email) return reply.code(401).send({ error: 'unauthorized' });

    const invites = await prisma.groupInvite.findMany({
      where: {
        kind: 'EMAIL',
        email: record.email,
        acceptedAt: null,
        declinedAt: null,
        OR: [
          { expiresAt: null },
          {
            expiresAt: {
              gt: new Date()
            }
          }
        ]
      },
      include: {
        group: { select: { name: true } },
        createdByUser: { select: { id: true, email: true } }
      },
      orderBy: { createdAt: 'desc' }
    });

    reply.send(
      invites.map((invite: any) => ({
        id: invite.id,
        groupId: invite.groupId,
        groupName: invite.group?.name ?? null,
        inviterUserId: invite.createdByUser?.id ?? null,
        inviterEmail: invite.createdByUser?.email ?? null,
        expiresAt: invite.expiresAt
      }))
    );
  });

  // Create group invite
  const createGroupInviteSchema = z.object({
    groupId: z.string(),
    kind: z.enum(['LINK', 'EMAIL']),
    email: z.string().email().optional(),
    expiresAt: z.string().datetime().optional()
  });

  fastify.post('/group-invites', async (req, reply) => {
    const user = (req as any).user;
    if (!user?.id) return reply.code(401).send({ error: 'unauthorized' });

    const parsed = createGroupInviteSchema.safeParse(req.body);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });

    const { groupId, kind, email, expiresAt } = parsed.data;

    const targetGroup = await prisma.group.findUnique({ where: { id: groupId } });
    if (!targetGroup) return reply.code(404).send({ error: 'group_not_found' });

    // Verify user has access to the group
    const membership = await prisma.groupMember.findFirst({
      where: { userId: user.id, groupId }
    });
    if (!membership) {
      // In early bootstrap cases the caller may hold a tenant-scoped token but lack an explicit membership record.
      // If the JWT already scopes the caller to this group, treat them as the initial member so they can issue invites.
      if (user.groupId === groupId) {
        await prisma.groupMember.create({ data: { userId: user.id, groupId } });
      } else {
        return reply.code(403).send({ error: 'forbidden' });
      }
    }

    if (kind === 'EMAIL' && !email) {
      return reply.code(400).send({ error: 'email_required_for_email_kind' });
    }

    const code = crypto.randomBytes(10).toString('hex');
    let parsedExpiresAt: Date | null = null;
    if (expiresAt) {
      parsedExpiresAt = new Date(expiresAt);
      if (Number.isNaN(parsedExpiresAt.getTime())) {
        return reply.code(400).send({ error: 'invalid_expires_at' });
      }
    }

    const invite = await prisma.groupInvite.create({
      data: {
        groupId,
        kind,
        email: kind === 'EMAIL' ? email : null,
        code,
        expiresAt: parsedExpiresAt,
        createdBy: user.id
      }
    });

    reply.code(201).send({ code: invite.code, expiresAt: invite.expiresAt });
  });

  // Get pending group invites
  fastify.get('/group-invites/pending', async (req, reply) => {
    const user = (req as any).user;
    if (!user?.id) return reply.code(401).send({ error: 'unauthorized' });

    const { email } = req.query as { email?: string };
    const userEmail = email || (await prisma.user.findUnique({ where: { id: user.id } }))?.email;
    if (!userEmail) return reply.code(401).send({ error: 'unauthorized' });

    const invites = await prisma.groupInvite.findMany({
      where: {
        email: userEmail,
        acceptedAt: null,
        declinedAt: null,
        OR: [
          { expiresAt: null },
          { expiresAt: { gt: new Date() } }
        ]
      },
      include: {
        group: { select: { name: true } }
      },
      orderBy: { createdAt: 'desc' }
    });

    reply.send(
      invites.map((invite: any) => ({
        id: invite.id,
        code: invite.code,
        groupId: invite.groupId,
        groupName: invite.group?.name ?? null,
        expiresAt: invite.expiresAt
      }))
    );
  });

  // Accept group invite
  fastify.post('/group-invites/:code/accept', async (req, reply) => {
    const user = (req as any).user;
    if (!user?.id) return reply.code(401).send({ error: 'unauthorized' });

    const { code } = req.params as { code: string };
    const invite = await prisma.groupInvite.findUnique({ where: { code } });
    if (!invite) return reply.code(404).send({ error: 'invite_not_found' });

    if (invite.acceptedAt) return reply.code(409).send({ error: 'already_accepted' });
    if (invite.declinedAt) return reply.code(409).send({ error: 'already_declined' });
    if (invite.expiresAt && invite.expiresAt.getTime() <= Date.now()) {
      return reply.code(410).send({ error: 'expired' });
    }

    // For EMAIL invites, verify email matches
    if (invite.kind === 'EMAIL' && invite.email) {
      const userRecord = await prisma.user.findUnique({ where: { id: user.id } });
      if (userRecord?.email !== invite.email) {
        return reply.code(403).send({ error: 'email_mismatch' });
      }
    }

    // Create membership if not exists
    const existingMembership = await prisma.groupMember.findFirst({
      where: { userId: user.id, groupId: invite.groupId }
    });

    if (!existingMembership) {
      await prisma.groupMember.create({
        data: {
          userId: user.id,
          groupId: invite.groupId
        }
      });
    }

    // Mark invite as accepted
    await prisma.groupInvite.update({
      where: { id: invite.id },
      data: {
        acceptedAt: new Date(),
        acceptedBy: user.id
      }
    });

    reply.send({ accepted: true });
  });
}
