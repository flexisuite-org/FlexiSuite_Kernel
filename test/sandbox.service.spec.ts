import { createSandboxForGroup } from '../src/lib/sandbox';
import { createTenantSeed } from './helpers/seed';
import { prisma } from '../src/lib/db';

describe('sandbox service', () => {
  it('duplicates the group and records a sandbox session', async () => {
    const suffix = 'sandbox-service';
    const { groupId } = await createTenantSeed(suffix);
    const ttlHours = 2;

    const { sandboxGroup, session } = await createSandboxForGroup({
      sourceGroupId: groupId,
      appId: 'app-service',
      ttlHours
    });

    expect(sandboxGroup.id).not.toBe(groupId);
    expect(sandboxGroup.name).toBe(`[sandbox] G-${suffix}`);
    expect(sandboxGroup.type).toBe('ORG');
    expect(session.sourceGroupId).toBe(groupId);
    expect(session.sandboxGroupId).toBe(sandboxGroup.id);
    expect(session.appId).toBe('app-service');
    expect(session.expiresAt).toBeTruthy();

    const deltaMs = session.expiresAt!.getTime() - Date.now();
    expect(deltaMs).toBeGreaterThan(ttlHours * 60 * 60 * 1000 - 1000);
    expect(deltaMs).toBeLessThan(ttlHours * 60 * 60 * 1000 + 60_000);

    const persisted = await prisma.group.findUnique({ where: { id: sandboxGroup.id } });
    expect(persisted).toBeTruthy();

    const persistedSession = await prisma.sandboxSession.findUnique({ where: { id: session.id } });
    expect(persistedSession?.sandboxGroupId).toBe(sandboxGroup.id);
  });
});
