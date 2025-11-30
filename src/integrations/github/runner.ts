import { Job } from 'bullmq';
import { promises as fs, existsSync } from 'fs';
import path from 'path';
import os from 'os';
import { promisify } from 'util';
import { exec } from 'child_process';
import semver from 'semver';
import { GithubBuildJobData } from './types';
import { updateStatus } from './status-store';
import { logger } from '../../lib/logger';
import { setRequestContext } from '../../lib/request-context';
import { prisma, setRlsContext } from '../../lib/db';
import { hashJson, sha256Hex, verifyIntegrity, stableStringify } from '../../lib/integrity';
import { bundleStorage } from '../../kernel/components/storage';
import { config } from '../../config';
import { signHmac, verifyHmac } from '../../lib/signature';
import { resolveToLock, ManifestFetcher } from '../../kernel/components/resolver';

const execAsync = promisify(exec);

function sanitizeMessage(msg: string) {
  return msg.length > 500 ? msg.slice(0, 500) + '...' : msg;
}

async function runCommand(command: string, cwd: string) {
  const env = {
    ...process.env,
    GIT_TERMINAL_PROMPT: '0'
  };
  const { stdout, stderr } = await execAsync(command, { cwd, env, maxBuffer: 10 * 1024 * 1024 });
  return { stdout, stderr };
}

async function zipArtifact(targetPath: string, tmpDir: string) {
  const stats = await fs.stat(targetPath);
  if (stats.isFile() && targetPath.toLowerCase().endsWith('.zip')) {
    return targetPath;
  }
  const outPath = path.join(tmpDir, 'artifact.zip');
  if (stats.isDirectory()) {
    await runCommand(`zip -qr ${outPath} .`, targetPath);
  } else {
    // single file -> place at root of zip
    await runCommand(`zip -qj ${outPath} ${path.basename(targetPath)}`, path.dirname(targetPath));
  }
  return outPath;
}

function resolveRepoUrl(repo: string) {
  const token = process.env.GITHUB_TOKEN;
  if (!token) return repo;
  if (!repo.startsWith('http')) return repo;
  try {
    const url = new URL(repo);
    if (url.hostname.includes('github.com') && !url.username) {
      url.username = 'x-access-token';
      url.password = token;
      return url.toString();
    }
  } catch (err) {
    logger.warn({ err, repo }, 'failed to parse repo url, using as-is');
  }
  return repo;
}

async function ensurePolicy(policyId?: string) {
  if (policyId) return policyId;
  const existing = await prisma.componentPolicy.findFirst();
  if (existing) return existing.id;
  const created = await prisma.componentPolicy.create({
    data: {
      name: 'default-policy',
      memoryMb: config.sandbox.memoryMb,
      timeoutMs: config.sandbox.timeoutMs,
      allowNetwork: false,
      allowedModules: [],
      executionMode: 'API'
    }
  });
  return created.id;
}

async function ensurePackage(data: GithubBuildJobData) {
  const existing = await prisma.componentPackage.findFirst({
    where: { name: data.packageName, version: data.version, ownerGroupId: data.groupId }
  });
  if (existing) return existing;

  const policyId = await ensurePolicy(data.policyId);
  const manifest = {
    name: data.packageName,
    version: data.version,
    engine: '1.0.0',
    capabilities: []
  };
  const integrity = hashJson(manifest);

  return prisma.componentPackage.create({
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

async function uploadBundle(pkgId: string, buffer: Buffer, groupId: string) {
  const bundleIntegrity = sha256Hex(buffer);
  const pkg = await prisma.componentPackage.findFirst({ where: { id: pkgId, ownerGroupId: groupId } });
  if (!pkg) throw new Error('package_not_found');

  const signingPayload = JSON.stringify({
    manifestIntegrity: pkg.integrityHash,
    bundleIntegrity
  });
  const signature = config.SIGNING_SECRET ? signHmac(signingPayload, config.SIGNING_SECRET) : undefined;

  const updated = await prisma.componentPackage.updateMany({
    where: { id: pkgId, ownerGroupId: groupId },
    data: { bundleIntegrity, bundleSignature: signature }
  });
  if (updated.count === 0) throw new Error('package_update_failed');
  await bundleStorage.save(pkgId, buffer);
  return { bundleIntegrity, bundleSignature: signature };
}

async function maybeApprove(pkgId: string, groupId: string) {
  await prisma.componentPackage.updateMany({
    where: { id: pkgId, ownerGroupId: groupId },
    data: { status: 'APPROVED', approvedAt: new Date() }
  });
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

async function maybeInstall(pkgId: string, groupId: string, userId?: string | null) {
  const target = await prisma.componentPackage.findFirst({ where: { id: pkgId, ownerGroupId: groupId } });
  if (!target) throw new Error('package_not_found');
  if (target.status !== 'APPROVED') throw new Error('package_not_approved');
  const fetcher = await makeFetcher(groupId, false);
  const root = await fetcher(target.name, target.version);
  const lock = await resolveToLock(root, fetcher, {});
  await prisma.componentInstall.upsert({
    where: { packageId_groupId_channel: { packageId: pkgId, groupId, channel: 'STABLE' } },
    update: { lockData: lock as any, installedBy: userId ?? undefined },
    create: {
      packageId: pkgId,
      groupId,
      channel: 'STABLE',
      lockData: lock as any,
      installedBy: userId ?? undefined
    }
  });
}

async function recordAudit(data: GithubBuildJobData, action: string, success: boolean, metadata: Record<string, any>) {
  await prisma.auditLog.create({
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

export async function processGithubBuildJob(job: Job<GithubBuildJobData>) {
  const data = job.data;
  setRequestContext({ groupId: data.groupId, userId: data.userId ?? null });
  await setRlsContext(data.groupId, data.userId ?? null, 'stable');

  await updateStatus(data.jobId, {
    status: 'cloning',
    message: `cloning ${data.repo}#${data.branch}`,
    repo: data.repo,
    branch: data.branch,
    artifactPath: data.artifactPath,
    groupId: data.groupId,
    userId: data.userId
  });

  const workDir = await fs.mkdtemp(path.join(os.tmpdir(), 'github-build-'));
  try {
    const repoUrl = resolveRepoUrl(data.repo);
    const cloneDir = path.join(workDir, 'repo');
    await runCommand(`git clone --depth 1 --branch ${data.branch} ${repoUrl} ${cloneDir}`, workDir);

    await updateStatus(data.jobId, {
      status: 'building',
      message: 'running build command',
      groupId: data.groupId,
      userId: data.userId
    });
    await runCommand(data.buildCommand, cloneDir);

    await updateStatus(data.jobId, {
      status: 'bundling',
      message: `bundling from ${data.artifactPath}`,
      groupId: data.groupId,
      userId: data.userId
    });

    const artifactPath = path.resolve(cloneDir, data.artifactPath);
    if (!existsSync(artifactPath)) {
      throw new Error(`artifact_path_not_found:${data.artifactPath}`);
    }
    const zipPath = await zipArtifact(artifactPath, workDir);
    const buffer = await fs.readFile(zipPath);

    await updateStatus(data.jobId, {
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

    await updateStatus(data.jobId, {
      status: 'done',
      message: 'completed',
      packageId: pkg.id,
      groupId: data.groupId,
      userId: data.userId
    });

    await recordAudit(data, 'done', true, { jobId: data.jobId, packageId: pkg.id, bundleIntegrity: upload.bundleIntegrity });
  } catch (err: any) {
    const message = sanitizeMessage(err?.message || 'build_failed');
    logger.error({ err, jobId: data.jobId }, 'github build failed');
    await updateStatus(data.jobId, {
      status: 'failed',
      error: message,
      message: 'failed',
      groupId: data.groupId,
      userId: data.userId
    });
    await recordAudit(data, 'failed', false, { jobId: data.jobId, error: message });
    throw err;
  } finally {
    await fs.rm(workDir, { recursive: true, force: true }).catch(() => {});
  }
}
