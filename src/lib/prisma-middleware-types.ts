/**
 * Prisma middleware types for version 5.9.0
 *
 * These types are defined manually because Prisma.MiddlewareParams
 * is not exported in this version. This approach provides:
 * 1. Type safety without depending on unstable internal types
 * 2. Forward compatibility with future Prisma versions
 * 3. Clear documentation of what the middleware actually uses
 */

export type PrismaAction =
  | 'findUnique'
  | 'findUniqueOrThrow'
  | 'findFirst'
  | 'findFirstOrThrow'
  | 'findMany'
  | 'create'
  | 'createMany'
  | 'createManyAndReturn'
  | 'update'
  | 'updateMany'
  | 'upsert'
  | 'delete'
  | 'deleteMany'
  | 'executeRaw'
  | 'queryRaw'
  | 'aggregate'
  | 'count'
  | 'runCommandRaw'
  | 'findRaw'
  | 'aggregateRaw'
  | 'groupBy';

export interface PrismaMiddlewareParams {
  /** The model name (e.g., 'User', 'Post'), or undefined for raw queries */
  model?: string;
  /** The operation being performed */
  action: PrismaAction;
  /** Arguments passed to the operation */
  args: any;
  /** Path to the data being accessed */
  dataPath: string[];
  /** Whether this operation runs in a transaction */
  runInTransaction: boolean;
}

export type PrismaMiddlewareNext = (
  params: PrismaMiddlewareParams
) => Promise<unknown>;

export type PrismaMiddleware = (
  params: PrismaMiddlewareParams,
  next: PrismaMiddlewareNext
) => Promise<unknown>;
