"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.processGithubBuildJob = processGithubBuildJob;
const fs_1 = require("fs");
const path_1 = __importDefault(require("path"));
const os_1 = __importDefault(require("os"));
const util_1 = require("util");
const child_process_1 = require("child_process");
const semver_1 = __importDefault(require("semver"));
const status_store_1 = require("./status-store");
const logger_1 = require("../../lib/logger");
const request_context_1 = require("../../lib/request-context");
const db_1 = require("../../lib/db");
const integrity_1 = require("../../lib/integrity");
const storage_1 = require("../../kernel/components/storage");
const config_1 = require("../../config");
const signature_1 = require("../../lib/signature");
const resolver_1 = require("../../kernel/components/resolver");
const execAsync = (0, util_1.promisify)(child_process_1.exec);
function sanitizeMessage(msg) {
    return msg.length > 500 ? msg.slice(0, 500) + '...' : msg;
}
async function runCommand(command, cwd) {
    const env = {
        ...process.env,
        GIT_TERMINAL_PROMPT: '0'
    };
    const { stdout, stderr } = await execAsync(command, { cwd, env, maxBuffer: 10 * 1024 * 1024 });
    return { stdout, stderr };
}
async function zipArtifact(targetPath, tmpDir) {
    const stats = await fs_1.promises.stat(targetPath);
    if (stats.isFile() && targetPath.toLowerCase().endsWith('.zip')) {
        return targetPath;
    }
    const outPath = path_1.default.join(tmpDir, 'artifact.zip');
    if (stats.isDirectory()) {
        await runCommand(`zip -qr ${outPath} .`, targetPath);
    }
    else {
        // single file -> place at root of zip
        await runCommand(`zip -qj ${outPath} ${path_1.default.basename(targetPath)}`, path_1.default.dirname(targetPath));
    }
    return outPath;
}
function resolveRepoUrl(repo) {
    const token = process.env.GITHUB_TOKEN;
    if (!token)
        return repo;
    if (!repo.startsWith('http'))
        return repo;
    try {
        const url = new URL(repo);
        if (url.hostname.includes('github.com') && !url.username) {
            url.username = 'x-access-token';
            url.password = token;
            return url.toString();
        }
    }
    catch (err) {
        logger_1.logger.warn({ err, repo }, 'failed to parse repo url, using as-is');
    }
    return repo;
}
async function ensurePolicy(policyId) {
    if (policyId)
        return policyId;
    const existing = await db_1.prisma.componentPolicy.findFirst();
    if (existing)
        return existing.id;
    const created = await db_1.prisma.componentPolicy.create({
        data: {
            name: 'default-policy',
            memoryMb: config_1.config.sandbox.memoryMb,
            timeoutMs: config_1.config.sandbox.timeoutMs,
            allowNetwork: false,
            allowedModules: [],
            executionMode: 'API'
        }
    });
    return created.id;
}
async function ensurePackage(data) {
    const existing = await db_1.prisma.componentPackage.findFirst({
        where: { name: data.packageName, version: data.version, ownerGroupId: data.groupId }
    });
    if (existing)
        return existing;
    const policyId = await ensurePolicy(data.policyId);
    const manifest = {
        name: data.packageName,
        version: data.version,
        engine: '1.0.0',
        capabilities: []
    };
    const integrity = (0, integrity_1.hashJson)(manifest);
    return db_1.prisma.componentPackage.create({
        data: {
            name: data.packageName,
            version: data.version,
            status: 'DRAFT',
            integrityHash: integrity,
            manifest,
            policyId,
            ownerGroupId: data.groupId,
            createdById: data.userId ?? undefined
        }
    });
}
async function uploadBundle(pkgId, buffer, groupId) {
    const bundleIntegrity = (0, integrity_1.sha256Hex)(buffer);
    const pkg = await db_1.prisma.componentPackage.findFirst({ where: { id: pkgId, ownerGroupId: groupId } });
    if (!pkg)
        throw new Error('package_not_found');
    const signingPayload = JSON.stringify({
        manifestIntegrity: pkg.integrityHash,
        bundleIntegrity
    });
    const signature = config_1.config.SIGNING_SECRET ? (0, signature_1.signHmac)(signingPayload, config_1.config.SIGNING_SECRET) : undefined;
    const updated = await db_1.prisma.componentPackage.updateMany({
        where: { id: pkgId, ownerGroupId: groupId },
        data: { bundleIntegrity, bundleSignature: signature }
    });
    if (updated.count === 0)
        throw new Error('package_update_failed');
    await storage_1.bundleStorage.save(pkgId, buffer);
    return { bundleIntegrity, bundleSignature: signature };
}
async function maybeApprove(pkgId, groupId) {
    await db_1.prisma.componentPackage.updateMany({
        where: { id: pkgId, ownerGroupId: groupId },
        data: { status: 'APPROVED', approvedAt: new Date() }
    });
}
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
async function maybeInstall(pkgId, groupId, userId) {
    const target = await db_1.prisma.componentPackage.findFirst({ where: { id: pkgId, ownerGroupId: groupId } });
    if (!target)
        throw new Error('package_not_found');
    if (target.status !== 'APPROVED')
        throw new Error('package_not_approved');
    const fetcher = await makeFetcher(groupId, false);
    const root = await fetcher(target.name, target.version);
    const lock = await (0, resolver_1.resolveToLock)(root, fetcher, {});
    await db_1.prisma.componentInstall.upsert({
        where: { packageId_groupId_channel: { packageId: pkgId, groupId, channel: 'STABLE' } },
        update: { lockData: lock, installedBy: userId ?? undefined },
        create: {
            packageId: pkgId,
            groupId,
            channel: 'STABLE',
            lockData: lock,
            installedBy: userId ?? undefined
        }
    });
}
async function recordAudit(data, action, success, metadata) {
    await db_1.prisma.auditLog.create({
        data: {
            groupId: data.groupId,
            actorUserId: data.userId ?? undefined,
            resource: 'github.build',
            action,
            metadata,
            success
        }
    });
}
async function processGithubBuildJob(job) {
    const data = job.data;
    (0, request_context_1.setRequestContext)({ groupId: data.groupId, userId: data.userId ?? null });
    await (0, db_1.setRlsContext)(data.groupId, data.userId ?? null, 'stable');
    await (0, status_store_1.updateStatus)(data.jobId, {
        status: 'cloning',
        message: `cloning ${data.repo}#${data.branch}`,
        repo: data.repo,
        branch: data.branch,
        artifactPath: data.artifactPath,
        groupId: data.groupId,
        userId: data.userId
    });
    const workDir = await fs_1.promises.mkdtemp(path_1.default.join(os_1.default.tmpdir(), 'github-build-'));
    try {
        const repoUrl = resolveRepoUrl(data.repo);
        const cloneDir = path_1.default.join(workDir, 'repo');
        await runCommand(`git clone --depth 1 --branch ${data.branch} ${repoUrl} ${cloneDir}`, workDir);
        await (0, status_store_1.updateStatus)(data.jobId, {
            status: 'building',
            message: 'running build command',
            groupId: data.groupId,
            userId: data.userId
        });
        await runCommand(data.buildCommand, cloneDir);
        await (0, status_store_1.updateStatus)(data.jobId, {
            status: 'bundling',
            message: `bundling from ${data.artifactPath}`,
            groupId: data.groupId,
            userId: data.userId
        });
        const artifactPath = path_1.default.resolve(cloneDir, data.artifactPath);
        if (!(0, fs_1.existsSync)(artifactPath)) {
            throw new Error(`artifact_path_not_found:${data.artifactPath}`);
        }
        const zipPath = await zipArtifact(artifactPath, workDir);
        const buffer = await fs_1.promises.readFile(zipPath);
        await (0, status_store_1.updateStatus)(data.jobId, {
            status: 'uploading',
            message: 'uploading bundle',
            groupId: data.groupId,
            userId: data.userId
        });
        const pkg = await ensurePackage(data);
        const upload = await uploadBundle(pkg.id, buffer, data.groupId);
        await recordAudit(data, 'upload', true, { jobId: data.jobId, packageId: pkg.id, bundleIntegrity: upload.bundleIntegrity });
        if (data.approve) {
            await maybeApprove(pkg.id, data.groupId);
        }
        if (data.install) {
            await maybeInstall(pkg.id, data.groupId, data.userId);
        }
        await (0, status_store_1.updateStatus)(data.jobId, {
            status: 'done',
            message: 'completed',
            packageId: pkg.id,
            groupId: data.groupId,
            userId: data.userId
        });
        await recordAudit(data, 'done', true, { jobId: data.jobId, packageId: pkg.id, bundleIntegrity: upload.bundleIntegrity });
    }
    catch (err) {
        const message = sanitizeMessage(err?.message || 'build_failed');
        logger_1.logger.error({ err, jobId: data.jobId }, 'github build failed');
        await (0, status_store_1.updateStatus)(data.jobId, {
            status: 'failed',
            error: message,
            message: 'failed',
            groupId: data.groupId,
            userId: data.userId
        });
        await recordAudit(data, 'failed', false, { jobId: data.jobId, error: message });
        throw err;
    }
    finally {
        await fs_1.promises.rm(workDir, { recursive: true, force: true }).catch(() => { });
    }
}
//# sourceMappingURL=runner.js.map