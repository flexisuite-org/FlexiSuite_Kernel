import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import { Prisma, GroupInviteKind } from '@prisma/client';
import { prisma } from '../../lib/db';
import { z } from 'zod';

const groupTypeEnum = z.enum(['ORG', 'TEAM', 'PROJECT']);

const createGroupSchema = z.object({
  name: z.string().min(1),
  type: groupTypeEnum,
  parentId: z.string().optional(),
  settings: z.record(z.unknown()).optional()
});

const updateGroupSchema = z.object({
  name: z.string().min(1).optional(),
  type: groupTypeEnum.optional(),
  parentId: z.string().nullable().optional(),
  settings: z.record(z.unknown()).optional()
});

const usersQuerySchema = z.object({
  email: z.string().email().optional(),
  groupId: z.string().optional(),
  status: z.enum(['active', 'deleted']).optional(),
  limit: z.string().optional()
});

const updateUserSchema = z.object({
  email: z.string().email().optional(),
  membershipRoleUpdates: z
    .array(
      z.object({
        membershipId: z.string(),
        roleIds: z.array(z.string())
      })
    )
    .optional()
});

const rolesQuerySchema = z.object({
  groupId: z.string()
});

const createRoleSchema = z.object({
  name: z.string().min(1),
  groupId: z.string(),
  permissionIds: z.array(z.string()).optional()
});

const rolePermissionsSchema = z.object({
  permissionIds: z.array(z.string())
});

const roleGroupQuerySchema = z.object({
  groupId: z.string()
});

const accountInviteQuerySchema = z.object({
  email: z.string().email().optional(),
  initialGroupId: z.string().optional(),
  status: z.enum(['used', 'unused']).optional(),
  expiresBefore: z.string().optional(),
  expiresAfter: z.string().optional(),
  limit: z.string().optional()
});

const groupInviteQuerySchema = z.object({
  groupId: z.string().optional(),
  email: z.string().email().optional(),
  kind: z.enum([GroupInviteKind.LINK, GroupInviteKind.EMAIL]).optional(),
  status: z.enum(['pending', 'accepted', 'declined', 'expired']).optional(),
  limit: z.string().optional()
});

const auditQuerySchema = z.object({
  from: z.string().optional(),
  to: z.string().optional(),
  groupId: z.string().optional(),
  userId: z.string().optional(),
  resource: z.string().optional(),
  action: z.string().optional(),
  success: z.enum(['true', 'false']).optional(),
  limit: z.string().optional(),
  cursor: z.string().optional()
});

function parseLimit(raw?: string, fallback = 50, max = 200) {
  if (!raw) return fallback;
  const parsed = Number.parseInt(raw, 10);
  if (Number.isNaN(parsed)) return fallback;
  return Math.min(Math.max(parsed, 1), max);
}

function parseDate(raw?: string) {
  if (!raw) return undefined;
  const parsed = new Date(raw);
  if (Number.isNaN(parsed.getTime())) return undefined;
  return parsed;
}

function requireAdmin(req: FastifyRequest, reply: FastifyReply) {
  const user = (req as any).user;
  if (!user) {
    reply.code(401).send({ error: 'unauthorized' });
    return null;
  }
  const roles: string[] = Array.isArray(user.roles) ? user.roles : [];
  if (!roles.includes('kernel-admin')) {
    reply.code(403).send({
      error: 'forbidden',
      message:
        'kernel-admin role required (TODO: replace with capability system or user-level kernel admin flag)'
    });
    return null;
  }
  return user;
}

async function fetchUserWithMemberships(userId: string) {
  return prisma.user.findUnique({
    where: { id: userId },
    include: {
      memberships: {
        include: {
          group: { select: { id: true, name: true, type: true } },
          roles: {
            include: {
              permissions: {
                include: {
                  permission: true
                }
              }
            }
          }
        }
      }
    }
  });
}

async function fetchRoleDetails(roleId: string, groupId: string) {
  return prisma.role.findFirst({
    where: { id: roleId, groupId },
    include: {
      group: { select: { id: true, name: true } },
      permissions: {
        include: {
          permission: true
        }
      }
    }
  });
}

interface IdParams {
  id: string;
}

export default async function adminRoutes(fastify: FastifyInstance) {
  fastify.post('/groups', async (req, reply) => {
    const user = requireAdmin(req, reply);
    if (!user) return;
    const parsed = createGroupSchema.safeParse(req.body);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const { name, type, parentId, settings } = parsed.data;
    const created = await prisma.group.create({
      data: {
        name,
        type,
        parentId,
        settings: (settings as Prisma.InputJsonValue) ?? undefined
      }
    });
    reply.code(201).send(created);
  });

  fastify.get('/groups', async (req, reply) => {
    const user = requireAdmin(req, reply);
    if (!user) return;
    const parsed = z
      .object({
        name: z.string().optional(),
        type: groupTypeEnum.optional(),
        parentId: z.string().optional(),
        limit: z.string().optional()
      })
      .safeParse(req.query);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const limit = parseLimit(parsed.data.limit, 50, 200);
    const where: Prisma.GroupWhereInput = {};
    if (parsed.data.name) where.name = { contains: parsed.data.name, mode: 'insensitive' };
    if (parsed.data.type) where.type = parsed.data.type;
    if (parsed.data.parentId) where.parentId = parsed.data.parentId;
    const items = await prisma.group.findMany({
      where,
      orderBy: { createdAt: 'desc' },
      take: limit
    });
    reply.send({ items, total: items.length });
  });

  fastify.patch('/groups/:id', async (req, reply) => {
    const user = requireAdmin(req, reply);
    if (!user) return;
    const parsed = updateGroupSchema.safeParse(req.body);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const updates: Record<string, unknown> = {};
    if (parsed.data.name) updates.name = parsed.data.name;
    if (parsed.data.type) updates.type = parsed.data.type;
    if (parsed.data.parentId !== undefined) updates.parentId = parsed.data.parentId;
    if (parsed.data.settings !== undefined) updates.settings = parsed.data.settings as Prisma.InputJsonValue;
    if (!Object.keys(updates).length) {
      return reply.code(400).send({ error: 'nothing_to_update' });
    }
    const updated = await prisma.group.update({
      where: { id: (req.params as IdParams).id },
      data: updates
    });
    reply.send(updated);
  });

  fastify.post('/groups/:id/deactivate', async (req, reply) => {
    const user = requireAdmin(req, reply);
    if (!user) return;
    const group = await prisma.group.findUnique({ where: { id: (req.params as IdParams).id } });
    if (!group) return reply.code(404).send({ error: 'not_found' });
    const currentSettings = (group.settings as Record<string, unknown>) ?? {};
    const updated = await prisma.group.update({
      where: { id: group.id },
      data: {
        settings: {
          ...currentSettings,
          adminDisabled: true
        }
      }
    });
    // TODO: wire actual lifecycle/deactivate logic (notifications, install cleanup, etc.).
    reply.send(updated);
  });

  fastify.get('/users', async (req, reply) => {
    const user = requireAdmin(req, reply);
    if (!user) return;
    const parsed = usersQuerySchema.safeParse(req.query);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const limit = parseLimit(parsed.data.limit);
    const where: Prisma.UserWhereInput = {};
    if (parsed.data.email) {
      where.email = { contains: parsed.data.email, mode: 'insensitive' };
    }
    if (parsed.data.status === 'active') where.deletedAt = null;
    if (parsed.data.status === 'deleted') where.deletedAt = { not: null };
    if (parsed.data.groupId) {
      where.memberships = { some: { groupId: parsed.data.groupId } };
    }
    const users = await prisma.user.findMany({
      where,
      orderBy: { createdAt: 'desc' },
      take: limit,
      include: {
        memberships: {
          include: {
            group: { select: { id: true, name: true, type: true } },
            roles: {
              include: {
                permissions: {
                  include: {
                    permission: true
                  }
                }
              }
            }
          }
        }
      }
    });
    reply.send({ items: users, total: users.length });
  });

  fastify.get('/users/:id', async (req, reply) => {
    const user = requireAdmin(req, reply);
    if (!user) return;
    const target = await fetchUserWithMemberships((req.params as IdParams).id);
    if (!target) return reply.code(404).send({ error: 'not_found' });
    reply.send(target);
  });

  fastify.patch('/users/:id', async (req, reply) => {
    const admin = requireAdmin(req, reply);
    if (!admin) return;
    const parsed = updateUserSchema.safeParse(req.body);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    if (parsed.data.email) {
      // TODO: consider locking/verification when changing primary email; kernel admin should tread carefully.
      await prisma.user.update({ where: { id: (req.params as IdParams).id }, data: { email: parsed.data.email } });
    }
    if (parsed.data.membershipRoleUpdates) {
      for (const update of parsed.data.membershipRoleUpdates) {
        const membership = await prisma.groupMember.findFirst({
          where: { id: update.membershipId },
          select: { groupId: true }
        });
        if (!membership) {
          return reply.code(404).send({ error: 'membership_not_found', membershipId: update.membershipId });
        }
        await prisma.groupMember.update({
          where: { id: update.membershipId },
          data: {
            roles: {
              set: Array.from(new Set(update.roleIds)).map((roleId) => ({ id: roleId }))
            }
          }
        });
      }
    }
    const refreshed = await fetchUserWithMemberships((req.params as IdParams).id);
    reply.send(refreshed);
  });

  fastify.post('/users/:id/force-logout', async (req, reply) => {
    const admin = requireAdmin(req, reply);
    if (!admin) return;
    const revoked = await prisma.refreshToken.updateMany({
      where: { userId: (req.params as IdParams).id },
      data: { revoked: true }
    });
    await prisma.auditLog.create({
      data: {
        actorUserId: admin.id,
        groupId: admin.groupId,
        resource: 'admin.users',
        action: 'force_logout',
        metadata: { targetUserId: (req.params as IdParams).id, revoked: revoked.count },
        success: true
      }
    });
    reply.send({ revoked: revoked.count });
  });

  fastify.get('/roles', async (req, reply) => {
    const user = requireAdmin(req, reply);
    if (!user) return;
    const parsed = rolesQuerySchema.safeParse(req.query);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const roles = await prisma.role.findMany({
      where: { groupId: parsed.data.groupId },
      include: {
        group: { select: { id: true, name: true } },
        permissions: {
          include: {
            permission: true
          }
        }
      }
    });
    reply.send({ items: roles });
  });

  fastify.post('/roles', async (req, reply) => {
    const user = requireAdmin(req, reply);
    if (!user) return;
    const parsed = createRoleSchema.safeParse(req.body);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const created = await prisma.$transaction(async (tx) => {
      const role = await tx.role.create({
        data: {
          name: parsed.data.name,
          groupId: parsed.data.groupId
        }
      });
      if (parsed.data.permissionIds) {
        await tx.rolePermission.createMany({
          data: parsed.data.permissionIds.map((permissionId) => ({
            roleId: role.id,
            permissionId
          }))
        });
      }
      return fetchRoleDetails(role.id, parsed.data.groupId);
    });
    if (!created) return reply.code(500).send({ error: 'role_creation_failed' });
    reply.code(201).send(created);
  });

  fastify.post('/roles/:id/permissions', async (req, reply) => {
    const user = requireAdmin(req, reply);
    if (!user) return;
    const parsed = rolePermissionsSchema.safeParse(req.body);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const groupQuery = roleGroupQuerySchema.safeParse(req.query);
    if (!groupQuery.success) return reply.code(400).send({ error: 'invalid_input', details: groupQuery.error.flatten() });
    const { groupId } = groupQuery.data;
    const role = await prisma.role.findFirst({
      where: { id: (req.params as IdParams).id, groupId },
      select: { groupId: true }
    });
    if (!role) return reply.code(404).send({ error: 'role_not_found' });
    await prisma.$transaction(async (tx) => {
      await tx.rolePermission.deleteMany({ where: { roleId: (req.params as IdParams).id } });
      if (parsed.data.permissionIds.length) {
        await tx.rolePermission.createMany({
          data: parsed.data.permissionIds.map((permissionId) => ({
            roleId: (req.params as IdParams).id,
            permissionId
          }))
        });
      }
    });
    const refreshed = await fetchRoleDetails((req.params as IdParams).id, groupId);
    if (!refreshed) return reply.code(500).send({ error: 'role_fetch_failed' });
    reply.send(refreshed);
  });

  fastify.get('/invites/accounts', async (req, reply) => {
    const user = requireAdmin(req, reply);
    if (!user) return;
    const parsed = accountInviteQuerySchema.safeParse(req.query);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const where: Prisma.AccountInviteWhereInput = {};
    if (parsed.data.email) where.email = { contains: parsed.data.email, mode: 'insensitive' };
    if (parsed.data.initialGroupId) where.initialGroupId = parsed.data.initialGroupId;
    if (parsed.data.status === 'used') where.usedAt = { not: null };
    if (parsed.data.status === 'unused') where.usedAt = null;
    if (parsed.data.expiresBefore) {
      const parsedDate = parseDate(parsed.data.expiresBefore);
      if (parsedDate) where.expiresAt = { lte: parsedDate };
    }
    if (parsed.data.expiresAfter) {
      const parsedDate = parseDate(parsed.data.expiresAfter);
      if (parsedDate) {
        where.expiresAt =
          where.expiresAt && typeof where.expiresAt === 'object' && !(where.expiresAt instanceof Date)
            ? { ...where.expiresAt, gte: parsedDate }
            : { gte: parsedDate };
      }
    }
    const limit = parseLimit(parsed.data.limit, 50, 200);
    const items = await prisma.accountInvite.findMany({
      where,
      include: {
        initialGroup: { select: { id: true, name: true } }
      },
      orderBy: { createdAt: 'desc' },
      take: limit
    });
    reply.send({ items, total: items.length });
  });

  fastify.get('/invites/groups', async (req, reply) => {
    const user = requireAdmin(req, reply);
    if (!user) return;
    const parsed = groupInviteQuerySchema.safeParse(req.query);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const where: Prisma.GroupInviteWhereInput = {};
    const now = new Date();
    if (parsed.data.groupId) where.groupId = parsed.data.groupId;
    if (parsed.data.email) where.email = { contains: parsed.data.email, mode: 'insensitive' };
    if (parsed.data.kind) where.kind = parsed.data.kind;
    if (parsed.data.status) {
      if (parsed.data.status === 'pending') {
        where.acceptedAt = null;
        where.declinedAt = null;
        where.AND = [
          {
            OR: [
              { expiresAt: null },
              { expiresAt: { gt: now } }
            ]
          }
        ];
      }
      if (parsed.data.status === 'accepted') {
        where.acceptedAt = { not: null };
      }
      if (parsed.data.status === 'declined') {
        where.declinedAt = { not: null };
      }
      if (parsed.data.status === 'expired') {
        where.expiresAt = { lt: now };
      }
    }
    const limit = parseLimit(parsed.data.limit, 50, 200);
    const items = await prisma.groupInvite.findMany({
      where,
      include: {
        group: { select: { id: true, name: true } }
      },
      orderBy: { createdAt: 'desc' },
      take: limit
    });
    reply.send({ items, total: items.length });
  });

  fastify.get('/audit-logs', async (req, reply) => {
    const user = requireAdmin(req, reply);
    if (!user) return;
    const parsed = auditQuerySchema.safeParse(req.query);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    const limit = parseLimit(parsed.data.limit, 50, 200);
    const where: Prisma.AuditLogWhereInput = {};
    if (parsed.data.groupId) where.groupId = parsed.data.groupId;
    if (parsed.data.userId) where.actorUserId = parsed.data.userId;
    if (parsed.data.resource) where.resource = parsed.data.resource;
    if (parsed.data.action) where.action = parsed.data.action;
    if (parsed.data.success) where.success = parsed.data.success === 'true';
    if (parsed.data.from || parsed.data.to || parsed.data.cursor) {
      where.createdAt = {};
      if (parsed.data.from) {
        const parsedDate = parseDate(parsed.data.from);
        if (parsedDate) where.createdAt.gte = parsedDate;
      }
      if (parsed.data.to) {
        const parsedDate = parseDate(parsed.data.to);
        if (parsedDate) where.createdAt.lte = parsedDate;
      }
      if (parsed.data.cursor) {
        const parsedDate = parseDate(parsed.data.cursor);
        if (parsedDate) where.createdAt.lt = parsedDate;
      }
    }
    const items = await prisma.auditLog.findMany({
      where,
      orderBy: { createdAt: 'desc' },
      take: limit + 1
    });
    let nextCursor: string | undefined;
    if (items.length > limit) {
      nextCursor = items[limit - 1].createdAt.toISOString();
      items.pop();
    }
    reply.send({ items, nextCursor, limit });
  });
}
