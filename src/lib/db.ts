import { PrismaClient, Prisma } from '@prisma/client';
import { logger } from './logger';
import { getRequestContext } from './request-context';

export const prisma = new PrismaClient({
  log: ['error', 'warn']
});

// Infer middleware types directly from PrismaClient.$use signature
// This approach is version-agnostic and doesn't depend on internal Prisma types
type PrismaMiddlewareFn = Parameters<typeof prisma.$use>[0];

// Set Postgres session variables for RLS per request.
export async function setRlsContext(groupId: string | null, userId: string | null, mode: 'draft' | 'stable' = 'stable') {
  const group = groupId ?? null;
  const user = userId ?? null;
  try {
    // Keep RLS aware of the current tenant/user and make draft requests read-only at the Postgres level.
    await prisma.$executeRawUnsafe(
      "SELECT set_config('flexi.current_group', $1, true), set_config('flexi.current_user', $2, true), set_config('default_transaction_read_only', $3, true)",
      group,
      user,
      mode === 'draft' ? 'on' : 'off'
    );
  } catch (err) {
    logger.warn({ err, group, user }, 'failed to set RLS context');
  }
}

// Run a callback within a single connection where RLS/session settings are applied.
export async function withRlsContext<T>(
  groupId: string | null,
  userId: string | null,
  mode: 'draft' | 'stable',
  fn: (tx: Prisma.TransactionClient) => Promise<T>
): Promise<T> {
  return prisma.$transaction(async (tx) => {
    await tx.$executeRawUnsafe(
      "SELECT set_config('flexi.current_group', $1, true), set_config('flexi.current_user', $2, true), set_config('default_transaction_read_only', $3, true)",
      groupId ?? null,
      userId ?? null,
      mode === 'draft' ? 'on' : 'off'
    );
    return fn(tx);
  });
}

// Prisma middleware to enforce group scoping for multi-tenant models.
const GROUP_SCOPED_FIELDS: Record<string, string> = {
  GroupMember: 'groupId',
  Role: 'groupId',
  Permission: 'groupId',
  AppInstall: 'groupId',
  EntityRecord: 'groupId',
  ComponentInstall: 'groupId',
  PlaygroundLog: 'groupId'
};

const OWNER_SCOPED_FIELDS: Record<string, string> = {
  ComponentPackage: 'ownerGroupId'
};

const multiTenantMiddleware: PrismaMiddlewareFn = async (params, next) => {
    const ctx = getRequestContext();
    const groupId = ctx?.groupId || null;
    const mode = ctx?.mode || 'stable';

    const field =
      GROUP_SCOPED_FIELDS[params.model ?? ''] ??
      OWNER_SCOPED_FIELDS[params.model ?? ''];

    if (field) {
      const isOwnerScoped = OWNER_SCOPED_FIELDS[params.model ?? ''] === field;
      const isReadAction =
        params.action === 'findUnique' ||
        params.action === 'findFirst' ||
        params.action === 'findMany';

      let effectiveGroupId = groupId || null;

      // Fallback: derive groupId from args when context is missing (e.g., background jobs).
      if (!effectiveGroupId) {
        const data = (params.args as any)?.data;
        const fromData = Array.isArray(data) ? data[0]?.[field] : data?.[field];
        const fromWhere = (params.args as any)?.where?.[field];
        effectiveGroupId = (fromData as string | null) || (fromWhere as string | null) || effectiveGroupId;
      }

      // If still missing, allow owner-scoped reads to proceed without scoping; otherwise block.
      if (!effectiveGroupId) {
        if (!(isOwnerScoped && isReadAction)) {
          throw new Error('missing groupId in request context');
        }
      }

      params.args ??= {};

      // Write operations should stamp the group (requires a resolved groupId)
      if (['create', 'createMany', 'upsert'].includes(params.action as string)) {
        if (mode === 'draft' && params.model !== 'PlaygroundLog') {
          throw new Error('write_not_allowed_in_draft');
        }
        if (!effectiveGroupId) throw new Error('missing groupId in request context');

        const assign = (data: unknown) => {
          if (data && typeof data === 'object') {
            (data as Record<string, unknown>)[field] = effectiveGroupId;
          }
        };

        if (Array.isArray(params.args.data)) {
          params.args.data.forEach(assign);
        } else {
          assign(params.args.data);
        }
      }

      // Read/update/delete operations should be scoped when we have a groupId.
      const scopedActions = [
        'findUnique',
        'findFirst',
        'findMany',
        'update',
        'updateMany',
        'delete',
        'deleteMany',
        'upsert'
      ];

      if (scopedActions.includes(params.action as string) && effectiveGroupId) {
        if (
          mode === 'draft' &&
          ['update', 'updateMany', 'delete', 'deleteMany', 'upsert'].includes(
            params.action as string
          ) &&
          params.model !== 'PlaygroundLog'
        ) {
          throw new Error('write_not_allowed_in_draft');
        }

        params.args.where = {
          ...(params.args.where || {}),
          [field]: effectiveGroupId
        };
      }
    }

    return next(params);
};

prisma.$use(multiTenantMiddleware);
