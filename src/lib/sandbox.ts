import { prisma } from './db';
import { Group, SandboxSession } from '@prisma/client';

export interface CreateSandboxOptions {
  sourceGroupId: string;
  appId?: string;
  ttlHours?: number;
}

export interface SandboxCreationResult {
  sandboxGroup: Group;
  session: SandboxSession;
}

export async function createSandboxForGroup(options: CreateSandboxOptions): Promise<SandboxCreationResult> {
  const { sourceGroupId, appId, ttlHours } = options;
  if (!sourceGroupId) {
    throw new Error('missing_source_group_id');
  }

  const sourceGroup = await prisma.group.findUnique({ where: { id: sourceGroupId } });
  if (!sourceGroup) {
    throw new Error('source_group_not_found');
  }

  const resolvedTtl = typeof ttlHours === 'number' && ttlHours > 0 ? ttlHours : 24;
  const expiresAt = resolvedTtl > 0 ? new Date(Date.now() + resolvedTtl * 60 * 60 * 1000) : null;

  const sandboxGroup = await prisma.group.create({
    data: {
      name: `[sandbox] ${sourceGroup.name}`,
      type: sourceGroup.type,
      parentId: sourceGroup.parentId,
      settings: sourceGroup.settings ?? undefined
    }
  });

  const session = await prisma.sandboxSession.create({
    data: {
      sourceGroupId,
      sandboxGroupId: sandboxGroup.id,
      appId,
      expiresAt
    }
  });

  // TODO: copy installs/entity records/playground data for the sandbox group based on appId and copy rules.

  return { sandboxGroup, session };
}
