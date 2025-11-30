import { Queue, Worker } from 'bullmq';
import IORedis from 'ioredis';
import { config } from '../../config';
import { logger } from '../../lib/logger';
import { GithubBuildJobData } from './types';
import { processGithubBuildJob } from './runner';
import { updateStatus } from './status-store';

let connection: IORedis | null = null;
let queue: Queue<GithubBuildJobData> | null = null;
let worker: Worker<GithubBuildJobData> | null = null;

function getConnection() {
  if (!connection) {
    connection = new IORedis(config.REDIS_URL, { maxRetriesPerRequest: null });
    connection.on('error', (err) => logger.error({ err }, 'redis connection error (github queue)'));
  }
  return connection;
}

export function getGithubBuildQueue() {
  if (!queue) {
    queue = new Queue<GithubBuildJobData>('github-build', {
      connection: getConnection(),
      defaultJobOptions: { removeOnComplete: true, attempts: 1 }
    });
  }
  return queue;
}

export function ensureGithubBuildWorker() {
  if (worker) return worker;
  worker = new Worker<GithubBuildJobData>('github-build', processGithubBuildJob, {
    connection: getConnection(),
    concurrency: 1
  });
  worker.on('failed', async (job, err) => {
    if (!job) return;
    await updateStatus(job.data.jobId, {
      status: 'failed',
      error: err?.message || 'job_failed',
      message: 'failed'
    });
  });
  return worker;
}

export async function shutdownGithubBuildQueue() {
  if (worker) {
    await worker.close().catch(() => {});
    worker = null;
  }
  if (queue) {
    await queue.close().catch(() => {});
    queue = null;
  }
  if (connection) {
    await connection.quit().catch(() => {});
    connection = null;
  }
}
