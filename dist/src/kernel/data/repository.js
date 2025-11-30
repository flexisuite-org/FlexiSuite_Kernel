"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.entityRepository = exports.EntityRepository = void 0;
const db_1 = require("../../lib/db");
const validator_1 = require("./validator");
class EntityRepository {
    async create(definitionId, groupId, schema, data) {
        const validator = (0, validator_1.getValidator)(definitionId, schema);
        if (!validator(data)) {
            const msg = validator.errors?.map((e) => `${e.instancePath} ${e.message}`).join(', ');
            throw new Error(`Validation failed: ${msg}`);
        }
        const record = await db_1.prisma.entityRecord.create({
            data: {
                definitionId,
                groupId,
                data,
                schemaVersion: schema.version ?? 1
            }
        });
        return record;
    }
    async findById(id, groupId) {
        return db_1.prisma.entityRecord.findFirst({ where: { id, groupId } });
    }
    async update(id, groupId, schema, data) {
        const validator = (0, validator_1.getValidator)(id, schema);
        if (!validator(data)) {
            const msg = validator.errors?.map((e) => `${e.instancePath} ${e.message}`).join(', ');
            throw new Error(`Validation failed: ${msg}`);
        }
        const result = await db_1.prisma.entityRecord.updateMany({ where: { id, groupId }, data: { data } });
        if (result.count === 0)
            throw new Error('Entity not found or not in tenant');
        return db_1.prisma.entityRecord.findFirst({ where: { id, groupId } });
    }
}
exports.EntityRepository = EntityRepository;
exports.entityRepository = new EntityRepository();
//# sourceMappingURL=repository.js.map