import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import { prisma } from '../../lib/db';
import { requestContext } from '../../lib/request-context';

// Minimal run/bundle placeholders using install lock

export default async function componentsRoutes(fastify: FastifyInstance) {
  fastify.get('/components/:id/bundle', async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId) return reply.code(401).send({ error: 'unauthorized' });
    const { id } = req.params as { id: string };
    const install = await prisma.componentInstall.findFirst({
      where: { id, groupId: ctx.groupId },
      include: { package: true }
    });
    if (!install) return reply.code(404).send({ error: 'not found' });
    reply.send({ manifest: install.package.manifest, integrity: install.package.integrityHash, lock: install.lockData });
  });

  fastify.post('/components/:id/run', async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId) return reply.code(401).send({ error: 'unauthorized' });
    const { id } = req.params as { id: string };
    const install = await prisma.componentInstall.findFirst({
      where: { id, groupId: ctx.groupId },
      include: { package: { include: { policy: true } } }
    });
    if (!install) return reply.code(404).send({ error: 'not found' });

    // integrity check: lock integrity vs stored hash (tamper detection)
    const lockIntegrity = (install.lockData as any)?.integrity;
    if (lockIntegrity && lockIntegrity !== install.package.integrityHash) {
      await prisma.auditLog.create({
        data: {
          actorUserId: ctx.userId,
          groupId: ctx.groupId,
          resource: 'component.run',
          action: 'integrity_mismatch',
          metadata: { installId: install.id, expected: install.package.integrityHash, got: lockIntegrity },
          success: false
        }
      });
      return reply.code(422).send({ error: 'integrity_mismatch' });
    }

    // APIモード固定: ユーザーコードはここでは実行しない。本番コンポーネントはフロント/別プロセスで動く前提。
    await prisma.auditLog.create({
      data: {
        actorUserId: ctx.userId,
        groupId: ctx.groupId,
        resource: 'component.run',
        action: 'api',
        metadata: { installId: install.id, packageId: install.packageId, mode: 'API' },
        success: true
      }
    });

    reply.send({ status: 'ok', mode: 'API', manifest: install.package.manifest, lock: install.lockData });
  });
}
