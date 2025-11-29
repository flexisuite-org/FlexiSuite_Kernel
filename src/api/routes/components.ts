import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import { prisma } from '../../lib/db';
import { requestContext } from '../../lib/request-context';
import { sandbox } from '../../kernel/runtime/sandbox';
import crypto from 'crypto';

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

    // integrity check: lock integrity vs stored hash
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

    const executionMode = install.package.policy.executionMode;
    const payload = req.body ?? {};
    let result: any;
    let success = true;

    if (executionMode === 'SANDBOX') {
      const script = (install.package.manifest as any)?.entryScript as string | undefined;
      if (!script) {
        success = false;
        result = { error: 'missing_entry_script' };
      } else {
        try {
          result = await sandbox.run(script, {
            kernel: { groupId: ctx.groupId, userId: ctx.userId, payload }
          });
        } catch (err: any) {
          success = false;
          result = { error: 'sandbox_error', message: err?.message };
        }
      }
    } else {
      // API mode: no user code execution, just echo and mark allowed
      result = { status: 'ok', mode: 'API', payload };
    }

    await prisma.auditLog.create({
      data: {
        actorUserId: ctx.userId,
        groupId: ctx.groupId,
        resource: 'component.run',
        action: executionMode?.toLowerCase() || 'api',
        metadata: { installId: install.id, packageId: install.packageId, mode: executionMode, success },
        success
      }
    });

    if (!success) return reply.code(500).send(result);
    reply.send(result);
  });
}
