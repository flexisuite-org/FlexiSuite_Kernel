"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.backfillEntityVersion = backfillEntityVersion;
const repository_1 = require("./repository");
const db_1 = require("../../lib/db");
// Example lazy migration helper: upgrades records where schemaVersion < targetVersion.
async function backfillEntityVersion(definitionId, targetVersion, transform) {
    const staleRecords = await db_1.prisma.entityRecord.findMany({ where: { definitionId, schemaVersion: { lt: targetVersion } }, take: 100 });
    for (const record of staleRecords) {
        const upgraded = transform(record.data);
        await repository_1.entityRepository.update(record.id, record.groupId, { version: targetVersion }, upgraded);
    }
}
//# sourceMappingURL=migration.js.map