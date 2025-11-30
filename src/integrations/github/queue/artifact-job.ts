import http from 'http';
import https from 'https';
import { URL } from 'url';
import { GithubBuildJobData } from '../../types';
import { prisma } from '../../../lib/db';
import { hashJson, sha256Hex } from '../../../lib/integrity';
import { signHmac } from '../../../lib/signature';
import { bundleStorage } from '../../../kernel/components/storage';
import { config } from '../../../config';
import { logger } from '../../../lib/logger';

const ARTIFACT_REDIRECT_LIMIT = 3;

interface PreparedManifest {
  manifest: Record<string, unknown>;
  integrity: string;
}

interface DependencyEntry {
  depName: string;
  depVersion: string;
  integrity?: string;
  kind: 'RUNTIME' | 'PEER' | 'OPTIONAL';
}

function prepareManifest(data: GithubBuildJobData): PreparedManifest {
  const baseManifest = data.manifest ? JSON.parse(JSON.stringify(data.manifest)) : {};
  const manifest = {
    engine: '1.0.0',
    capabilities: [],
    ...baseManifest,
    name: data.packageName,
    version: data.version
  } as Record<string, unknown>;
  if (!Array.isArray((manifest as any).capabilities)) {
    (manifest as any).capabilities = [];
  }
  const integrity = hashJson(manifest);
  return { manifest, integrity };
}

function collectDependencies(manifest: Record<string, unknown>): DependencyEntry[] {
  const groups: Array<[string, DependencyEntry['kind']]> = [
    ['dependencies', 'RUNTIME'],
    ['peerDependencies', 'PEER'],
    ['optionalDependencies', 'OPTIONAL']
  ];
  const entries: DependencyEntry[] = [];
  for (const [key, kind] of groups) {
    const list = manifest[key];
    if (!Array.isArray(list)) continue;
    for (const item of list as any[]) {
      if (!item || typeof item !== 'object') continue;
      const depName = (item as any).name as string | undefined;
      const depVersion = (item as any).version as string | undefined;
      if (!depName || !depVersion) continue;
      entries.push({
        depName,
        depVersion,
        integrity: (item as any).integrity,
        kind
      });
    }
  }
  return entries;
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

const redirectStatuses = new Set([301, 302, 303, 307, 308]);

async function downloadBufferFromUrl(urlStr: string, token?: string, remaining = ARTIFACT_REDIRECT_LIMIT): Promise<Buffer> {
  if (remaining <= 0) throw new Error('artifact_redirect_limit_exceeded');
  let parsedUrl: URL;
  try {
    parsedUrl = new URL(urlStr);
  } catch (err) {
    throw new Error('invalid_artifact_url');
  }
  const headers: Record<string, string> = {
    'user-agent': 'FlexiSuite-Kernel',
    accept: 'application/zip, application/octet-stream, */*'
  };
  if (token) {
    headers.authorization = `Bearer ${token}`;
  }

  const client = parsedUrl.protocol === 'https:' ? https : http;
  return new Promise<Buffer>((resolve, reject) => {
    const req = client.get(parsedUrl, { headers }, (res) => {
      const { statusCode } = res;
      if (statusCode && redirectStatuses.has(statusCode)) {
        const location = res.headers.location;
        if (!location) {
          reject(new Error('artifact_redirect_missing_location'));
          return;
        }
        res.resume();
        resolve(downloadBufferFromUrl(new URL(location, parsedUrl).toString(), token, remaining - 1));
        return;
      }
      if (statusCode && statusCode >= 400) {
        reject(new Error(`artifact_download_failed:${statusCode}`));
        return;
      }
      const chunks: Buffer[] = [];
      res.on('data', (chunk) => {
        chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
      });
      res.on('end', () => resolve(Buffer.concat(chunks)));
      res.on('error', reject);
    });
    req.on('error', reject);
  });
}

export async function downloadGithubArtifact(data: GithubBuildJobData) {
  if (!data.artifactUrl) {
    throw new Error('artifact_url_required');
  }
  try {
    const token = data.artifactToken || process.env.GITHUB_TOKEN;
    return await downloadBufferFromUrl(data.artifactUrl, token);
  } catch (err) {
    logger.error({ err, jobId: data.jobId, artifactUrl: data.artifactUrl }, 'failed to download github artifact');
    throw err;
  }
}

export interface ArtifactRegistrationResult {
  packageId: string;
  bundleIntegrity: string;
  bundleSignature?: string;
}

async function ensurePackageWithManifest(data: GithubBuildJobData, prepared: PreparedManifest) {
  const existing = await prisma.componentPackage.findFirst({
    where: { name: data.packageName, version: data.version, ownerGroupId: data.groupId }
  });
  if (existing) return existing;

  const policyId = await ensurePolicy(data.policyId);
  return prisma.$transaction(async (tx) => {
    const pkg = await tx.componentPackage.create({
      data: {
        name: data.packageName,
        version: data.version,
        status: 'DRAFT',
        integrityHash: prepared.integrity,
        manifest: prepared.manifest,
        policyId,
        ownerGroupId: data.groupId,
        createdById: data.userId ?? undefined
      }
    });
    const deps = collectDependencies(prepared.manifest);
    if (deps.length) {
      await tx.componentDependency.createMany({
        data: deps.map((entry) => ({
          packageId: pkg.id,
          depName: entry.depName,
          depVersion: entry.depVersion,
          integrity: entry.integrity,
          kind: entry.kind
        }))
      });
    }
    return pkg;
  });
}

export async function registerArtifactBundle(data: GithubBuildJobData, buffer: Buffer): Promise<ArtifactRegistrationResult> {
  const prepared = prepareManifest(data);
  const pkg = await ensurePackageWithManifest(data, prepared);
  const bundleIntegrity = sha256Hex(buffer);
  const signingPayload = JSON.stringify({ manifestIntegrity: pkg.integrityHash, bundleIntegrity });
  const signature = config.SIGNING_SECRET ? signHmac(signingPayload, config.SIGNING_SECRET) : undefined;

  const updated = await prisma.componentPackage.updateMany({
    where: { id: pkg.id, ownerGroupId: data.groupId },
    data: { bundleIntegrity, bundleSignature: signature }
  });
  if (updated.count === 0) {
    throw new Error('package_update_failed');
  }

  await bundleStorage.save(pkg.id, buffer);
  return { packageId: pkg.id, bundleIntegrity, bundleSignature: signature };
}

export async function processArtifactFlow(
  data: GithubBuildJobData,
  fetcher: (data: GithubBuildJobData) => Promise<Buffer> = downloadGithubArtifact,
  registrar: (data: GithubBuildJobData, buffer: Buffer) => Promise<ArtifactRegistrationResult> = registerArtifactBundle
): Promise<ArtifactRegistrationResult> {
  const buffer = await fetcher(data);
  return registrar(data, buffer);
}
