import { FastifyRequest, FastifyReply } from 'fastify';
import { prisma } from '../../lib/db';

export function requireAuth() {
  return async (req: FastifyRequest, reply: FastifyReply) => {
    if (!req.user) {
      reply.code(401).send({ error: 'unauthorized' });
    }
  };
}

export function authorize(resource: string, action: string) {
  return async (req: FastifyRequest, reply: FastifyReply) => {
    if (!req.user) return reply.code(401).send({ error: 'unauthorized' });
    const roles = (req.user as any).roles ?? [];

    const rolePermissions = await prisma.rolePermission.findMany({
      where: { role: { name: { in: roles }, groupId: (req.user as any).groupId } },
      include: { permission: true }
    });

    const allowed = rolePermissions.some((rp) => {
      const p = rp.permission;
      return p.resource === resource && (p.action === action || p.action === '*');
    });

    if (!allowed) return reply.code(403).send({ error: 'forbidden' });
  };
}
