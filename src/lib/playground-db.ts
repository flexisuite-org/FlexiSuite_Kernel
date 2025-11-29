import { prisma } from './db';

// Simple helper to route draft writes to a playground schema/table namespace.
// For now, we just tag data with isPlayground=true; later we can move to separate schema.

export async function saveDraftResult(groupId: string, userId: string | null, payload: any) {
  return prisma.auditLog.create({
    data: {
      actorUserId: userId ?? undefined,
      groupId,
      resource: 'sandbox.draft.write',
      action: 'store',
      metadata: { payload, isPlayground: true },
      success: true
    }
  });
}
