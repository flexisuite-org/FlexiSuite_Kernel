import { prisma } from '../../lib/db';

// Capability handlers are intentionally minimal and read-only by default.
export const capabilityHandlers: Record<string, (payload: any) => Promise<any> | any> = {
  'echo': async (payload) => ({ echo: payload }),
  'time.now': async () => ({ now: new Date().toISOString() }),
  'data.entity.get': async (payload) => {
    if (!payload?.id) return { error: 'id_required' };
    const rec = await prisma.entityRecord.findFirst({ where: { id: payload.id } });
    if (!rec) return { error: 'not_found' };
    return { id: rec.id, data: rec.data, schemaVersion: rec.schemaVersion };
  },
  'data.entity.list': async (payload) => {
    const { limit = 20, definitionId } = payload || {};
    const recs = await prisma.entityRecord.findMany({
      where: definitionId ? { definitionId } : {},
      take: Math.min(100, limit)
    });
    return { items: recs.map((r) => ({ id: r.id, definitionId: r.definitionId, data: r.data })) };
  },
  'data.entity.listByDefinition': async (payload) => {
    if (!payload?.definitionId) return { error: 'definitionId_required' };
    const { limit = 20 } = payload;
    const recs = await prisma.entityRecord.findMany({ where: { definitionId: payload.definitionId }, take: Math.min(100, limit) });
    return { items: recs.map((r) => ({ id: r.id, data: r.data })) };
  },
  'data.entity.getDefinition': async (payload) => {
    if (!payload?.definitionId) return { error: 'definitionId_required' };
    const def = await prisma.entityDefinition.findFirst({ where: { id: payload.definitionId } });
    if (!def) return { error: 'not_found' };
    return { id: def.id, name: def.name, version: def.version, schema: def.schema };
  }
};
