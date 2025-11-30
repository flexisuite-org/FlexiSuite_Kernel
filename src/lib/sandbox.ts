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

export type CloneableModel = 'EntityRecord' | 'AppInstall';

export interface CloneEntitySpec {
  model: CloneableModel;
  ids?: string[];
  whereJson?: unknown;
}

export interface CloneEntitiesResultItem {
  model: CloneableModel;
  requested: number;
  cloned: number;
  skipped: number;
}

export interface CloneEntitiesSummary {
  sessionId: string;
  sourceGroupId: string;
  sandboxGroupId: string;
  results: CloneEntitiesResultItem[];
}

export async function cloneEntitiesForSandboxSession(
  sessionId: string,
  specs: CloneEntitySpec[]
): Promise<CloneEntitiesSummary> {
  if (!Array.isArray(specs) || specs.length === 0) {
    throw new Error('no_specs');
  }

  const session = await prisma.sandboxSession.findUnique({
    where: { id: sessionId }
  });
  if (!session) {
    throw new Error('sandbox_session_not_found');
  }

  if (session.expiresAt && session.expiresAt.getTime() < Date.now()) {
    throw new Error('sandbox_session_expired');
  }

  const results = specs.map((spec) => {
    const requested = Array.isArray(spec.ids) ? spec.ids.length : 0;
    // TODO: implement actual copy logic per model and replace the placeholder tally.
    return {
      model: spec.model,
      requested,
      cloned: 0,
      skipped: requested
    };
  });

  return {
    sessionId: session.id,
    sourceGroupId: session.sourceGroupId,
    sandboxGroupId: session.sandboxGroupId,
    results
  };
}

export interface EnsureEntitiesRequest {
  sessionId: string;
  specs: CloneEntitySpec[];
}

export interface EnsureEntitiesResult extends CloneEntitiesSummary {}

export async function ensureEntitiesForSandboxSession(
  req: EnsureEntitiesRequest
): Promise<EnsureEntitiesResult> {
  return cloneEntitiesForSandboxSession(req.sessionId, req.specs);
}
