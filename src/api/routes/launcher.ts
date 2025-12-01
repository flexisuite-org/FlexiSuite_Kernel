import { FastifyInstance } from 'fastify';
import { prisma, withRlsContext } from '../../lib/db';

export default async function launcherRoutes(fastify: FastifyInstance) {
  fastify.get('/groups', async (req, reply) => {
    const user = (req as any).user;
    if (!user?.id) return reply.code(401).send({ error: 'unauthorized' });

    const memberships = await prisma.groupMember.findMany({
      where: { userId: user.id },
      select: { groupId: true }
    });

    const groups = [];
    for (const membership of memberships) {
      const payload = await withRlsContext(membership.groupId, user.id, 'stable', async (tx) => {
        const group = await tx.group.findUnique({
          where: { id: membership.groupId },
          select: { id: true, name: true, type: true }
        });

        const memberRecord = await tx.groupMember.findFirst({
          where: { groupId: membership.groupId, userId: user.id },
          include: { roles: { select: { name: true } } }
        });

        const installs = await tx.componentInstall.findMany({
          where: { groupId: membership.groupId },
          include: {
            package: {
              select: {
                id: true,
                name: true,
                version: true,
                status: true,
                ownerGroupId: true
              }
            }
          }
        });

        return { group, installs, memberRoles: memberRecord?.roles ?? [] };
      });

      groups.push({
        groupId: membership.groupId,
        name: payload.group?.name ?? null,
        type: payload.group?.type ?? null,
        roles: payload.memberRoles.map((role: any) => role.name),
        installs: payload.installs.map((install: any) => ({
          installId: install.id,
          packageId: install.packageId,
          channel: install.channel,
          status: install.package?.status ?? null,
          packageName: install.package?.name ?? null,
          packageVersion: install.package?.version ?? null
        }))
      });
    }

    reply.send(groups);
  });
}
