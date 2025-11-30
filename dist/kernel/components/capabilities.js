"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.capabilityHandlers = void 0;
const db_1 = require("../../lib/db");
// Capability handlers are intentionally minimal and read-only by default.
exports.capabilityHandlers = {
    'echo': async (payload) => ({ echo: payload }),
    'time.now': async () => ({ now: new Date().toISOString() }),
    'data.entity.get': async (payload) => {
        if (!payload?.id)
            return { error: 'id_required' };
        const rec = await db_1.prisma.entityRecord.findFirst({ where: { id: payload.id } });
        if (!rec)
            return { error: 'not_found' };
        return { id: rec.id, data: rec.data, schemaVersion: rec.schemaVersion };
    },
    'data.entity.list': async (payload) => {
        const { limit = 20, definitionId } = payload || {};
        const recs = await db_1.prisma.entityRecord.findMany({
            where: definitionId ? { definitionId } : {},
            take: Math.min(100, limit)
        });
        return { items: recs.map((r) => ({ id: r.id, definitionId: r.definitionId, data: r.data })) };
    },
    'data.entity.listByDefinition': async (payload) => {
        if (!payload?.definitionId)
            return { error: 'definitionId_required' };
        const { limit = 20 } = payload;
        const recs = await db_1.prisma.entityRecord.findMany({ where: { definitionId: payload.definitionId }, take: Math.min(100, limit) });
        return { items: recs.map((r) => ({ id: r.id, data: r.data })) };
    },
    'data.entity.getDefinition': async (payload) => {
        if (!payload?.definitionId)
            return { error: 'definitionId_required' };
        const def = await db_1.prisma.entityDefinition.findFirst({ where: { id: payload.definitionId } });
        if (!def)
            return { error: 'not_found' };
        return { id: def.id, name: def.name, version: def.version, schema: def.schema };
    },
    'data.entity.listDefinitions': async (payload) => {
        const { limit = 50 } = payload || {};
        const defs = await db_1.prisma.entityDefinition.findMany({ take: Math.min(200, limit) });
        return { items: defs.map((d) => ({ id: d.id, name: d.name, version: d.version })) };
    }
};
//# sourceMappingURL=capabilities.js.map