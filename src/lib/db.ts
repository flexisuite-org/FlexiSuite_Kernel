import { PrismaClient } from '@prisma/client';
import { logger } from './logger';

export const prisma = new PrismaClient({
  log: ['error', 'warn']
});

// Set Postgres session variables for RLS per request.
export async function setRlsContext(groupId: string | null, userId: string | null) {
  const group = groupId ?? null;
  const user = userId ?? null;
  try {
    await prisma.$executeRawUnsafe(
      "SELECT set_config('flexi.current_group', $1, true), set_config('flexi.current_user', $2, true)",
      group,
      user
    );
  } catch (err) {
    logger.warn({ err, group, user }, 'failed to set RLS context');
  }
}
