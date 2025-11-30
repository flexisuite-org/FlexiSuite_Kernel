/**
 * Custom Prisma middleware types
 *
 * These types are manually defined to avoid issues with Prisma's internal
 * type exports in version 5.9.0. This approach:
 * 1. Doesn't depend on Prisma.Middleware* types (which may not exist or be 'never')
 * 2. Defines only the fields we actually use in our middleware
 * 3. Provides proper type safety without implicit any errors
 */

export type PrismaAction =
  | 'findUnique'
  | 'findFirst'
  | 'findMany'
  | 'create'
  | 'createMany'
  | 'update'
  | 'updateMany'
  | 'upsert'
  | 'delete'
  | 'deleteMany';

export interface PrismaMiddlewareParams {
  /** The model name (e.g., 'User', 'Post'), or null/undefined for raw queries */
  model?: string | null;
  /** The operation being performed */
  action: PrismaAction | string; // Allow string for compatibility with future actions
  /** Arguments passed to the operation */
  args: {
    data?: unknown;
    where?: Record<string, unknown>;
    // Allow other fields that might be present
    [key: string]: unknown;
  };
  /** Path to the data being accessed (not used but included for completeness) */
  dataPath?: string[];
  /** Whether this operation runs in a transaction (not used but included for completeness) */
  runInTransaction?: boolean;
}

export type PrismaMiddlewareNext = (
  params: PrismaMiddlewareParams
) => Promise<unknown>;

export type PrismaMiddlewareFn = (
  params: PrismaMiddlewareParams,
  next: PrismaMiddlewareNext
) => Promise<unknown>;
