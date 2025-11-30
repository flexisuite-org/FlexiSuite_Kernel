/**
 * Type utilities for Prisma Client
 *
 * These types are extracted directly from PrismaClient method signatures
 * using TypeScript's Parameters<> utility. This approach:
 * 1. Doesn't depend on internal Prisma namespace types (which may not exist in 5.9.0)
 * 2. Is version-agnostic and follows Prisma's actual API
 * 3. Provides proper type safety without implicit any errors
 */

import type { PrismaClient } from '@prisma/client';

/**
 * Type of the transaction client passed to $transaction callbacks
 * Extracted from: prisma.$transaction(async (tx) => { ... })
 */
export type PrismaTransactionClient = Parameters<
  Parameters<PrismaClient['$transaction']>[0]
>[0];

/**
 * Type of middleware function passed to prisma.$use()
 * Extracted from: prisma.$use(async (params, next) => { ... })
 */
export type PrismaMiddlewareFn = Parameters<PrismaClient['$use']>[0];

/**
 * Type of the first parameter (params) in middleware function
 */
export type PrismaMiddlewareParams = Parameters<PrismaMiddlewareFn>[0];

/**
 * Type of the second parameter (next) in middleware function
 */
export type PrismaMiddlewareNext = Parameters<PrismaMiddlewareFn>[1];
