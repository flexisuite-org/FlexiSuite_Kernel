import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import { prisma } from '../../lib/db';
import { requestContext } from '../../lib/request-context';
import { ComponentManifest } from '../../kernel/components/types';
import { verifyIntegrity } from '../../lib/integrity';
import { verifyHmac } from '../../lib/signature';
import { config } from '../../config';
import { capabilityHandlers } from '../../kernel/components/capabilities';
import { z } from 'zod';

// Minimal run/bundle placeholders using install lock

export default async function componentsRoutes(fastify: FastifyInstance) {
  const runSchema = z.object({ payload: z.any().optional() });
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
    const parsedBody = runSchema.safeParse(req.body ?? {});
    if (!parsedBody.success) return reply.code(400).send({ error: 'invalid_input', details: parsedBody.error.flatten() });
    const inputPayload = parsedBody.data.payload;
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

    // verify manifest integrity & signature at runtime as well
    const manifestStr = JSON.stringify(install.package.manifest);
    if (!verifyIntegrity(install.package.integrityHash, manifestStr)) {
      await prisma.auditLog.create({
        data: {
          actorUserId: ctx.userId,
          groupId: ctx.groupId,
          resource: 'component.run',
          action: 'integrity_mismatch_manifest',
          metadata: { installId: install.id },
          success: false
        }
      });
      return reply.code(422).send({ error: 'integrity_mismatch_manifest' });
    }

    if ((install.package.manifest as any).signature && config.SIGNING_SECRET) {
      const sig = (install.package.manifest as any).signature as string;
      if (!verifyHmac(manifestStr, sig, config.SIGNING_SECRET)) {
        await prisma.auditLog.create({
          data: {
            actorUserId: ctx.userId,
            groupId: ctx.groupId,
            resource: 'component.run',
            action: 'signature_mismatch',
            metadata: { installId: install.id },
            success: false
          }
        });
        return reply.code(422).send({ error: 'signature_mismatch' });
      }
    }

    if (install.package.bundleIntegrity && config.SIGNING_SECRET) {
      const signingPayload = JSON.stringify({ manifestIntegrity: install.package.integrityHash, bundleIntegrity: install.package.bundleIntegrity });
      if (!install.package.bundleSignature || !verifyHmac(signingPayload, install.package.bundleSignature, config.SIGNING_SECRET)) {
        await prisma.auditLog.create({
          data: {
            actorUserId: ctx.userId,
            groupId: ctx.groupId,
            resource: 'component.run',
            action: 'bundle_signature_mismatch',
            metadata: { installId: install.id },
            success: false
          }
        });
        return reply.code(422).send({ error: 'bundle_signature_mismatch' });
      }
    }

    // APIモード: capabilities に基づき限定的な処理のみ実行
    const manifest = install.package.manifest as ComponentManifest;
    const allowed = (manifest.allowedCapabilities ?? manifest.capabilities ?? []).filter((cap) =>
      config.capabilityAllowlist.includes(cap)
    );
    const requested = allowed;
    const payload = inputPayload;
    const results: Record<string, any> = {};
    for (const cap of requested) {
      const handler = capabilityHandlers[cap];
      if (!handler) {
        results[cap] = { error: 'unsupported_capability' };
        continue;
      }
      try {
        results[cap] = await handler(payload);
      } catch (err: any) {
        results[cap] = { error: err?.message || 'capability_error' };
      }
    }

    await prisma.auditLog.create({
      data: {
        actorUserId: ctx.userId,
        groupId: ctx.groupId,
        resource: 'component.run',
        action: 'api',
        metadata: { installId: install.id, packageId: install.packageId, mode: 'API', capabilities: requested },
        success: true
      }
    });

    reply.send({ status: 'ok', mode: 'API', results, manifest: install.package.manifest, lock: install.lockData });
  });
}
