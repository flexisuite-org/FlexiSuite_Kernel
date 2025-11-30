import { getRedis } from '../../lib/redis';
import { logger } from '../../lib/logger';
import { publishWs } from '../../lib/ws-bus';
import { GithubBuildStatus } from './types';

const STATUS_TTL_SECONDS = 60 * 60 * 24; // keep status for 1 day

const statusKey = (jobId: string) => `github:job:${jobId}`;
const statusChannel = (jobId: string) => `job:${jobId}`;

export async function readStatus(jobId: string): Promise<GithubBuildStatus | null> {
  const raw = await getRedis().get(statusKey(jobId));
  if (!raw) return null;
  try {
    return JSON.parse(raw) as GithubBuildStatus;
  } catch (err) {
    logger.warn({ err, jobId }, 'failed to parse github job status');
    return null;
  }
}

export async function writeStatus(status: GithubBuildStatus) {
  const redis = getRedis();
  const payload = JSON.stringify(status);
  await redis.set(statusKey(status.jobId), payload, 'EX', STATUS_TTL_SECONDS);
  await redis.publish(statusChannel(status.jobId), payload);
  try {
    publishWs(statusChannel(status.jobId), status, { groupId: status.groupId });
  } catch (err) {
    logger.warn({ err }, 'failed to publish ws status');
  }
  return status;
}

export async function updateStatus(jobId: string, patch: Partial<GithubBuildStatus>) {
  const current = (await readStatus(jobId)) ?? ({} as GithubBuildStatus);
  const next: GithubBuildStatus = {
    ...current,
    ...patch,
    jobId,
    groupId: patch.groupId ?? current.groupId ?? 'unknown',
    updatedAt: new Date().toISOString()
  };
  return writeStatus(next);
}

export function channelForJob(jobId: string) {
  return statusChannel(jobId);
}
