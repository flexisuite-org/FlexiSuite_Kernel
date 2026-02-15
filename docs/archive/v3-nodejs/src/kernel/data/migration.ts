import { Queue, Worker } from 'bullmq';
import { entityRepository } from './repository';
import { prisma } from '../../lib/db';
import { redis } from '../../lib/redis';
import { logger } from '../../lib/logger';

const MIGRATION_QUEUE = 'flexi-migrations';

export interface MigrationJob {
  definitionId: string;
  targetVersion: number;
  batchSize: number;
}

export const migrationQueue = new Queue(MIGRATION_QUEUE, { connection: redis });

// The actual logic should be provided by the caller via a registry or similar.
// For now, we provide a generic worker setup.
export function startMigrationWorker(transformMap: Record<string, (data: any) => any>) {
  const worker = new Worker(
    MIGRATION_QUEUE,
    async (job) => {
      const { definitionId, targetVersion, batchSize } = job.data as MigrationJob;
      const transform = transformMap[definitionId];
      
      if (!transform) {
        throw new Error(`No transform found for definition ${definitionId}`);
      }

      const staleRecords = await prisma.entityRecord.findMany({
        where: { definitionId, schemaVersion: { lt: targetVersion } },
        take: batchSize
      });

      logger.info({ definitionId, count: staleRecords.length }, 'migrating batch');

      for (const record of staleRecords) {
        const upgraded = transform(record.data);
        await entityRepository.update(record.id, record.groupId, { version: targetVersion }, upgraded);
      }

      // If there are more records, queue another job for the next batch
      const remaining = await prisma.entityRecord.count({
        where: { definitionId, schemaVersion: { lt: targetVersion } }
      });

      if (remaining > 0) {
        await migrationQueue.add('backfill', job.data);
      }
    },
    { connection: redis, concurrency: 1 }
  );

  worker.on('failed', (job, err) => {
    logger.error({ jobId: job?.id, err }, 'migration job failed');
  });

  return worker;
}

export async function enqueueBackfill(definitionId: string, targetVersion: number, batchSize = 100) {
  await migrationQueue.add('backfill', { definitionId, targetVersion, batchSize });
}
