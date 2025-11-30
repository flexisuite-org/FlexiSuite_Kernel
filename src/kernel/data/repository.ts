import { Prisma } from '@prisma/client';
import { prisma } from '../../lib/db';
import { getValidator } from './validator';

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

    const result = await prisma.entityRecord.updateMany({ where: { id, groupId }, data: { data } });
    if (result.count === 0) throw new Error('Entity not found or not in tenant');
    return prisma.entityRecord.findFirst({ where: { id, groupId } });
  }
}

export const entityRepository = new EntityRepository();
