import { Prisma } from '@prisma/client';
import { prisma } from '../../lib/db';
import { getValidator } from './validator';
import { recordAuditLog } from '../../lib/audit';

export class EntityRepository {
  async create(definitionId: string, groupId: string, schema: object, data: Prisma.InputJsonValue) {
    const validator = getValidator(definitionId, schema);
    if (!validator(data)) {
      const msg = validator.errors?.map((e) => `${e.instancePath} ${e.message}`).join(', ');
      throw new Error(`Validation failed: ${msg}`);
    }

    const record = await prisma.entityRecord.create({
      data: {
        definitionId,
        groupId,
        data,
        schemaVersion: (schema as any).version ?? 1
      }
    });

    await recordAuditLog({
      resource: 'EntityRecord',
      action: 'create',
      metadata: { id: record.id, definitionId }
    });

    return record;
  }

  async findById(id: string, groupId: string) {
    return prisma.entityRecord.findFirst({ where: { id, groupId } });
  }

  async update(id: string, groupId: string, schema: object, data: Prisma.InputJsonValue) {
    const validator = getValidator(id, schema);
    if (!validator(data)) {
      const msg = validator.errors?.map((e) => `${e.instancePath} ${e.message}`).join(', ');
      throw new Error(`Validation failed: ${msg}`);
    }

    return prisma.$transaction(async (tx) => {
      const current = await tx.entityRecord.findFirst({ where: { id, groupId } });
      if (!current) throw new Error('Entity not found or not in tenant');

      // Create history record
      await tx.entityHistory.create({
        data: {
          entityId: id,
          data: current.data as any,
          version: current.schemaVersion
        }
      });

      // Update record
      const updated = await tx.entityRecord.update({
        where: { id },
        data: {
          data,
          schemaVersion: (schema as any).version ?? current.schemaVersion
        }
      });

      await recordAuditLog({
        resource: 'EntityRecord',
        action: 'update',
        metadata: { id, definitionId: current.definitionId }
      });

      return updated;
    });
  }
}

export const entityRepository = new EntityRepository();
