import { entityRepository } from './repository';
import { prisma } from '../../lib/db';

// Example lazy migration helper: upgrades records where schemaVersion < targetVersion.
export async function backfillEntityVersion(definitionId: string, targetVersion: number, transform: (data: any) => any) {
  const staleRecords = await prisma.entityRecord.findMany({ where: { definitionId, schemaVersion: { lt: targetVersion } }, take: 100 });

  for (const record of staleRecords) {
    const upgraded = transform(record.data);
    await entityRepository.update(record.id, record.groupId, { version: targetVersion }, upgraded);
  }
}
