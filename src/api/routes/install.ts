import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import { prisma } from '../../lib/db';
import { requestContext } from '../../lib/request-context';
import { resolveToLock, ManifestFetcher } from '../../kernel/components/resolver';
import { ComponentManifest } from '../../kernel/components/types';

interface InstallBody {
  packageId?: string;
  name?: string;
  version?: string;
  channel?: 'STABLE' | 'DRAFT';
}

async function makeFetcher(groupId: string): Promise<ManifestFetcher> {
  return async (name: string, range: string) => {
    // For now, resolve by exact version match; semver range is checked in resolver.
    const pkg = await prisma.componentPackage.findFirst({
      where: { name, version: range, ownerGroupId: groupId }
    });
    if (!pkg) throw new Error(`package not found ${name}@${range}`);

    const deps = await prisma.componentDependency.findMany({ where: { packageId: pkg.id } });
    const manifest: ComponentManifest = {
      ...(pkg.manifest as any),
      name: pkg.name,
      version: pkg.version,
      policyId: pkg.policyId,
      integrity: pkg.integrityHash,
      dependencies: deps.filter((d) => d.kind === 'RUNTIME').map((d) => ({ name: d.depName, version: d.depVersion, integrity: d.integrity || undefined })),
      peerDependencies: deps.filter((d) => d.kind === 'PEER').map((d) => ({ name: d.depName, version: d.depVersion, integrity: d.integrity || undefined })),
      optionalDependencies: deps.filter((d) => d.kind === 'OPTIONAL').map((d) => ({ name: d.depName, version: d.depVersion, integrity: d.integrity || undefined }))
    };

    return {
      manifest,
      integrity: pkg.integrityHash,
      resolved: pkg.id
    };
  };
}

export default async function installRoutes(fastify: FastifyInstance) {
  // Install package for current group
  fastify.post('/install', async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId || !ctx?.userId) return reply.code(401).send({ error: 'unauthorized' });

    const body = req.body as InstallBody;
    const channel = body.channel ?? 'STABLE';

    const target = body.packageId
      ? await prisma.componentPackage.findFirst({ where: { id: body.packageId, ownerGroupId: ctx.groupId } })
      : await prisma.componentPackage.findFirst({ where: { name: body.name ?? '', version: body.version ?? '', ownerGroupId: ctx.groupId } });

    if (!target) return reply.code(404).send({ error: 'package not found' });
    if (channel === 'STABLE' && target.status !== 'APPROVED') {
      return reply.code(409).send({ error: 'package not approved' });
    }

    const fetcher = await makeFetcher(ctx.groupId);
    const root = await fetcher(target.name, target.version);
    const lock = await resolveToLock(root, fetcher, {});

    const install = await prisma.componentInstall.create({
      data: {
        packageId: target.id,
        groupId: ctx.groupId,
        channel,
        lockData: lock,
        installedBy: ctx.userId
      }
    });

    reply.code(201).send({ installId: install.id, channel });
  });

  // List installs for group
  fastify.get('/install', async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId) return reply.code(401).send({ error: 'unauthorized' });
    const installs = await prisma.componentInstall.findMany({
      where: { groupId: ctx.groupId },
      include: { package: true }
    });
    reply.send(installs);
  });

  // Delete install
  fastify.delete('/install/:id', async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId) return reply.code(401).send({ error: 'unauthorized' });
    const { id } = req.params as { id: string };
    const deleted = await prisma.componentInstall.deleteMany({ where: { id, groupId: ctx.groupId } });
    if (deleted.count === 0) return reply.code(404).send({ error: 'not found' });
    reply.send({ id, deleted: true });
  });
}
