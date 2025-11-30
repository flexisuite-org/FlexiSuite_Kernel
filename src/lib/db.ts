import { PrismaClient, Prisma } from '@prisma/client';
import { logger } from './logger';
import { getRequestContext } from './request-context';

export const prisma = new PrismaClient({
  log: ['error', 'warn']
});

// Set Postgres session variables for RLS per request.
export async function setRlsContext(groupId: string | null, userId: string | null, mode: 'draft' | 'stable' = 'stable') {
  const group = groupId ?? null;
  const user = userId ?? null;
  try {
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

prisma.$use(async (params: Prisma.MiddlewareParams, next: (params: Prisma.MiddlewareParams) => Promise<any>) => {
  const ctx = getRequestContext();
  const groupId = ctx?.groupId || null;
  const mode = ctx?.mode || 'stable';

  const field = GROUP_SCOPED_FIELDS[params.model ?? ''] ?? OWNER_SCOPED_FIELDS[params.model ?? ''];
  if (field) {
    if (!groupId) throw new Error('missing groupId in request context');
    params.args ??= {};

    // Write operations should stamp the group
    if (['create', 'createMany', 'upsert'].includes(params.action)) {
      if (mode === 'draft' && params.model !== 'PlaygroundLog') {
        throw new Error('write_not_allowed_in_draft');
      }
      const assign = (data: any) => {
        if (data && typeof data === 'object') data[field] = groupId;
      };
      if (Array.isArray(params.args.data)) params.args.data.forEach(assign);
      else assign(params.args.data);
    }

    // Read/update/delete operations should be scoped
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
    if (scopedActions.includes(params.action)) {
      if (mode === 'draft' && ['update', 'updateMany', 'delete', 'deleteMany', 'upsert'].includes(params.action) && params.model !== 'PlaygroundLog') {
        throw new Error('write_not_allowed_in_draft');
      }
      params.args.where = { ...(params.args.where || {}), [field]: groupId };
    }
  }

  return next(params);
});
