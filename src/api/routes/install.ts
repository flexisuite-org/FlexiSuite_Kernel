import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import { prisma } from '../../lib/db';
import { requestContext } from '../../lib/request-context';
import { resolveToLock, ManifestFetcher } from '../../kernel/components/resolver';
import { ComponentManifest } from '../../kernel/components/types';
import semver from 'semver';
import { z } from 'zod';
import { verifyIntegrity, stableStringify } from '../../lib/integrity';
import { verifyHmac } from '../../lib/signature';
import { config } from '../../config';

interface InstallBody {
  packageId?: string;
  name?: string;
  version?: string;
  channel?: 'STABLE' | 'DRAFT';
}

async function makeFetcher(groupId: string, allowDraft: boolean): Promise<ManifestFetcher> {
  return async (name: string, range: string) => {
    const all = await prisma.componentPackage.findMany({
      where: { name, ownerGroupId: groupId, status: allowDraft ? undefined : 'APPROVED' },
      orderBy: { version: 'desc' }
    });
    const pkg = all.find((p) => semver.satisfies(p.version, range));
    if (!pkg) throw new Error(`package not found ${name}@${range}`);

    const deps = await prisma.componentDependency.findMany({ where: { packageId: pkg.id } });
    const baseManifest = pkg.manifest as any;
    const manifest: ComponentManifest = {
      ...baseManifest,
      name: pkg.name,
      version: pkg.version,
      policyId: pkg.policyId,
      integrity: pkg.integrityHash,
      dependencies: deps
        .filter((d) => d.kind === 'RUNTIME')
        .map((d) => ({ name: d.depName, version: d.depVersion, integrity: d.integrity || undefined })),
      peerDependencies: deps
        .filter((d) => d.kind === 'PEER')
        .map((d) => ({ name: d.depName, version: d.depVersion, integrity: d.integrity || undefined })),
      optionalDependencies: deps
        .filter((d) => d.kind === 'OPTIONAL')
        .map((d) => ({ name: d.depName, version: d.depVersion, integrity: d.integrity || undefined }))
    };

    // integrity check against stored hash of manifest JSON
    if (!verifyIntegrity(pkg.integrityHash, baseManifest)) {
      throw new Error(`integrity mismatch for ${name}@${pkg.version}`);
    }

    const manifestStr = stableStringify(baseManifest);
    if (manifest.signature && config.SIGNING_SECRET) {
      if (!verifyHmac(manifestStr, manifest.signature, config.SIGNING_SECRET)) {
        throw new Error(`signature mismatch for ${name}@${pkg.version}`);
      }
    }

    if (pkg.bundleIntegrity && config.SIGNING_SECRET) {
      const signingPayload = JSON.stringify({ manifestIntegrity: pkg.integrityHash, bundleIntegrity: pkg.bundleIntegrity });
      if (!pkg.bundleSignature || !verifyHmac(signingPayload, pkg.bundleSignature, config.SIGNING_SECRET)) {
        throw new Error(`bundle signature mismatch for ${name}@${pkg.version}`);
      }
    }

    return { manifest, integrity: pkg.integrityHash, resolved: pkg.id };
  };
}

export default async function installRoutes(fastify: FastifyInstance) {
  const installSchema = z.object({
    packageId: z.string().optional(),
    name: z.string().optional(),
    version: z.string().optional(),
    channel: z.enum(['STABLE', 'DRAFT']).optional()
  }).refine((v) => v.packageId || (v.name && v.version), { message: 'packageId or (name+version) required' });

  // Install package for current group
  fastify.post('/install', { config: { rateLimit: { max: 30, timeWindow: '1 minute' } } }, async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId || !ctx?.userId) return reply.code(401).send({ error: 'unauthorized' });

    const parsed = installSchema.safeParse(req.body);
    if (!parsed.success) return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });

    const body = parsed.data as InstallBody;
    const channel = body.channel ?? 'STABLE';

    const target = body.packageId
      ? await prisma.componentPackage.findFirst({ where: { id: body.packageId, ownerGroupId: ctx.groupId } })
      : await prisma.componentPackage.findFirst({ where: { name: body.name ?? '', version: body.version ?? '', ownerGroupId: ctx.groupId } });

    if (!target) return reply.code(404).send({ error: 'package not found' });
    if (channel === 'STABLE' && target.status !== 'APPROVED') {
      return reply.code(409).send({ error: 'package not approved' });
    }

    const fetcher = await makeFetcher(ctx.groupId, channel === 'DRAFT');
    const root = await fetcher(target.name, target.version);
    if (root.manifest.integrity !== target.integrityHash) {
      return reply.code(422).send({ error: 'integrity_mismatch_root' });
    }
    const lock = await resolveToLock(root, fetcher, {});

    const install = await prisma.componentInstall.create({
      data: {
        packageId: target.id,
        groupId: ctx.groupId,
        channel,
        lockData: lock as any,
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
