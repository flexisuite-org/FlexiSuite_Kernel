"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.default = installRoutes;
const db_1 = require("../../lib/db");
const request_context_1 = require("../../lib/request-context");
const resolver_1 = require("../../kernel/components/resolver");
const semver_1 = __importDefault(require("semver"));
const zod_1 = require("zod");
const integrity_1 = require("../../lib/integrity");
const signature_1 = require("../../lib/signature");
const config_1 = require("../../config");
async function makeFetcher(groupId, allowDraft) {
    return async (name, range) => {
        const all = await db_1.prisma.componentPackage.findMany({
            where: { name, ownerGroupId: groupId, status: allowDraft ? undefined : 'APPROVED' },
            orderBy: { version: 'desc' }
        });
        const pkg = all.find((p) => semver_1.default.satisfies(p.version, range));
        if (!pkg)
            throw new Error(`package not found ${name}@${range}`);
        const deps = await db_1.prisma.componentDependency.findMany({ where: { packageId: pkg.id } });
        const baseManifest = pkg.manifest;
        const manifest = {
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
        if (!(0, integrity_1.verifyIntegrity)(pkg.integrityHash, baseManifest)) {
            throw new Error(`integrity mismatch for ${name}@${pkg.version}`);
        }
        const manifestStr = (0, integrity_1.stableStringify)(baseManifest);
        if (manifest.signature && config_1.config.SIGNING_SECRET) {
            if (!(0, signature_1.verifyHmac)(manifestStr, manifest.signature, config_1.config.SIGNING_SECRET)) {
                throw new Error(`signature mismatch for ${name}@${pkg.version}`);
            }
        }
        if (pkg.bundleIntegrity && config_1.config.SIGNING_SECRET) {
            const signingPayload = JSON.stringify({ manifestIntegrity: pkg.integrityHash, bundleIntegrity: pkg.bundleIntegrity });
            if (!pkg.bundleSignature || !(0, signature_1.verifyHmac)(signingPayload, pkg.bundleSignature, config_1.config.SIGNING_SECRET)) {
                throw new Error(`bundle signature mismatch for ${name}@${pkg.version}`);
            }
        }
        return { manifest, integrity: pkg.integrityHash, resolved: pkg.id };
    };
}
async function installRoutes(fastify) {
    const installSchema = zod_1.z.object({
        packageId: zod_1.z.string().optional(),
        name: zod_1.z.string().optional(),
        version: zod_1.z.string().optional(),
        channel: zod_1.z.enum(['STABLE', 'DRAFT']).optional()
    }).refine((v) => v.packageId || (v.name && v.version), { message: 'packageId or (name+version) required' });
    // Install package for current group
    fastify.post('/install', { config: { rateLimit: { max: 30, timeWindow: '1 minute' } } }, async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        if (!ctx?.groupId || !ctx?.userId)
            return reply.code(401).send({ error: 'unauthorized' });
        const parsed = installSchema.safeParse(req.body);
        if (!parsed.success)
            return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
        const body = parsed.data;
        const channel = body.channel ?? 'STABLE';
        const target = body.packageId
            ? await db_1.prisma.componentPackage.findFirst({ where: { id: body.packageId, ownerGroupId: ctx.groupId } })
            : await db_1.prisma.componentPackage.findFirst({ where: { name: body.name ?? '', version: body.version ?? '', ownerGroupId: ctx.groupId } });
        if (!target)
            return reply.code(404).send({ error: 'package not found' });
        if (channel === 'STABLE' && target.status !== 'APPROVED') {
            return reply.code(409).send({ error: 'package not approved' });
        }
        const fetcher = await makeFetcher(ctx.groupId, channel === 'DRAFT');
        const root = await fetcher(target.name, target.version);
        if (root.manifest.integrity !== target.integrityHash) {
            return reply.code(422).send({ error: 'integrity_mismatch_root' });
        }
        const lock = await (0, resolver_1.resolveToLock)(root, fetcher, {});
        const install = await db_1.prisma.componentInstall.create({
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
    fastify.get('/install', async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        if (!ctx?.groupId)
            return reply.code(401).send({ error: 'unauthorized' });
        const installs = await db_1.prisma.componentInstall.findMany({
            where: { groupId: ctx.groupId },
            include: { package: true }
        });
        reply.send(installs);
    });
    // Delete install
    fastify.delete('/install/:id', async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        if (!ctx?.groupId)
            return reply.code(401).send({ error: 'unauthorized' });
        const { id } = req.params;
        const deleted = await db_1.prisma.componentInstall.deleteMany({ where: { id, groupId: ctx.groupId } });
        if (deleted.count === 0)
            return reply.code(404).send({ error: 'not found' });
        reply.send({ id, deleted: true });
    });
}
//# sourceMappingURL=install.js.map