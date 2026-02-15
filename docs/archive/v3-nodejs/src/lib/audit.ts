import { prisma } from './db';
import { getRequestContext } from './request-context';
import { logger } from './logger';

export interface AuditLogOptions {
  resource: string;
  action: string;
  metadata?: any;
  success?: boolean;
}

/**
 * Records an audit log entry based on the current request context.
 */
export async function recordAuditLog(opts: AuditLogOptions) {
  const ctx = getRequestContext();
  const actorUserId = ctx?.userId || null;
  const groupId = ctx?.groupId || null;

  try {
    const log = await prisma.auditLog.create({
      data: {
        actorUserId,
        groupId,
        resource: opts.resource,
        action: opts.action,
        metadata: opts.metadata || {},
        success: opts.success ?? true,
      }
    });
    return log;
  } catch (err) {
    // We log the error but don't throw to avoid breaking the main operation
    logger.error({ err, opts }, 'failed to record audit log');
    return null;
  }
}
