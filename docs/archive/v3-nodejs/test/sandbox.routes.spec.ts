import request from 'supertest';
import jwt from 'jsonwebtoken';
import { buildServer } from '../src/api/server';
import { createTenantSeed } from './helpers/seed';
import { config } from '../src/config';
import { prisma } from '../src/lib/db';

describe('sandbox routes', () => {
  const app = buildServer();

  beforeAll(async () => {
    await app.ready();
  });

  afterAll(async () => {
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  it('creates a sandbox group for the current context', async () => {
    const { groupId, userId } = await createTenantSeed('routes');
    const token = jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);

    const res = await request(app.server)
      .post('/sandbox/groups')
      .set('Authorization', `Bearer ${token}`)
      .send({ appId: 'app-route', ttlHours: 3 });

    expect(res.status).toBe(200);
    expect(res.body).toEqual(
      expect.objectContaining({
        sandboxGroupId: expect.any(String),
        sessionId: expect.any(String)
      })
    );

    const persisted = await prisma.sandboxSession.findUnique({ where: { id: res.body.sessionId } });
    expect(persisted).toBeTruthy();
    if (!persisted) return;

    expect(persisted.sourceGroupId).toBe(groupId);
    expect(persisted.sandboxGroupId).toBe(res.body.sandboxGroupId);
    expect(persisted.appId).toBe('app-route');
    expect(persisted.expiresAt).toBeTruthy();

    const deltaMs = persisted.expiresAt!.getTime() - Date.now();
    expect(deltaMs).toBeGreaterThan(3 * 60 * 60 * 1000 - 1000);
  });
});
