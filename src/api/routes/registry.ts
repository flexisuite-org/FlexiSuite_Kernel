import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import { prisma } from '../../lib/db';
import { requestContext } from '../../lib/request-context';
import { sha256Hex } from '../../lib/integrity';
import { signHmac } from '../../lib/signature';
import { config } from '../../config';

interface PackageInput {
  name: string;
  version: string;
  manifest: any;
  policyId: string;
  bundleIntegrity?: string;
  dependencies?: { name: string; version: string; integrity?: string; kind?: string }[];
}

export default async function registryRoutes(fastify: FastifyInstance) {
  // list packages for current group
  fastify.get('/packages', async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId) return reply.code(400).send({ error: 'groupId required' });
    const packages = await prisma.componentPackage.findMany({
      where: { ownerGroupId: ctx.groupId },
      include: { dependencies: true }
    });
    reply.send(packages);
  });

  // register draft package
  fastify.post('/packages', { config: { rateLimit: { max: 20, timeWindow: '1 minute' } } }, async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId || !ctx?.userId) return reply.code(401).send({ error: 'unauthorized' });
    const body = req.body as PackageInput;
    const manifestBase = body.manifest;
    const integrity = sha256Hex(JSON.stringify(manifestBase));
    const bundleIntegrity = body.bundleIntegrity;
    const signingPayload = JSON.stringify({ manifestIntegrity: integrity, bundleIntegrity });
    const signature = config.SIGNING_SECRET ? signHmac(signingPayload, config.SIGNING_SECRET) : undefined;

    const manifestToStore = { ...manifestBase, integrity, bundleIntegrity, signature };

    const result = await prisma.$transaction(async (tx) => {
      const pkg = await tx.componentPackage.create({
        data: {
          name: body.name,
          version: body.version,
          integrityHash: integrity,
          bundleIntegrity,
          bundleSignature: signature,
          manifest: manifestToStore,
          policyId: body.policyId,
          ownerGroupId: ctx.groupId!,
          createdById: ctx.userId!
        }
      });

      if (body.dependencies?.length) {
        await tx.componentDependency.createMany({
          data: body.dependencies.map((d) => ({
            packageId: pkg.id,
            depName: d.name,
            depVersion: d.version,
            integrity: d.integrity,
            kind: (d.kind as any) ?? 'RUNTIME'
          }))
        });
      }
      return pkg;
    });

    reply.code(201).send(result);
  });

  // update bundle integrity/signature after upload
  fastify.post('/packages/:id/bundle', async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId) return reply.code(401).send({ error: 'unauthorized' });
    const { id } = req.params as { id: string };
    const { bundleIntegrity } = req.body as { bundleIntegrity: string };
    if (!bundleIntegrity) return reply.code(400).send({ error: 'bundleIntegrity_required' });
    const signingPayload = JSON.stringify({ manifestIntegrity: undefined, bundleIntegrity });
    const signature = config.SIGNING_SECRET ? signHmac(signingPayload, config.SIGNING_SECRET) : undefined;
    const updated = await prisma.componentPackage.updateMany({
      where: { id, ownerGroupId: ctx.groupId },
      data: { bundleIntegrity, bundleSignature: signature }
    });
    if (updated.count === 0) return reply.code(404).send({ error: 'not found' });
    reply.send({ id, bundleIntegrity, bundleSignature: signature });
  });

  fastify.post('/packages/:id/approve', async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId) return reply.code(401).send({ error: 'unauthorized' });
    const { id } = req.params as { id: string };
    const updated = await prisma.componentPackage.updateMany({
      where: { id, ownerGroupId: ctx.groupId },
      data: { status: 'APPROVED', approvedAt: new Date() }
    });
    if (updated.count === 0) return reply.code(404).send({ error: 'not found' });
    reply.send({ id, status: 'APPROVED' });
  });

  fastify.post('/packages/:id/revoke', async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId) return reply.code(401).send({ error: 'unauthorized' });
    const { id } = req.params as { id: string };
    const updated = await prisma.componentPackage.updateMany({
      where: { id, ownerGroupId: ctx.groupId },
      data: { status: 'REVOKED' }
    });
    if (updated.count === 0) return reply.code(404).send({ error: 'not found' });
    reply.send({ id, status: 'REVOKED' });
  });
}
