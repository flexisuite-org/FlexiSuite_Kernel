import { prisma } from './db';

// Playground storage (non-prod). Writes go to PlaygroundLog with RLS enforced.
export async function saveDraftResult(groupId: string, userId: string | null, payload: any) {
  return prisma.playgroundLog.create({
    data: {
      groupId,
      userId: userId ?? undefined,
      payload
    }
  });
}
