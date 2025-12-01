import request from 'supertest';
import jwt from 'jsonwebtoken';
import { buildServer } from '../src/api/server';
import { config } from '../src/config';
import { createTenantSeed } from './helpers/seed';
import { prisma } from '../src/lib/db';

describe('GET /auth/me', () => {
  const app = buildServer();

  beforeAll(async () => {
    await app.ready();
  });

  afterAll(async () => {
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  it('returns the authenticated user and their memberships', async () => {
    const { groupId, userId } = await createTenantSeed('auth-me');
    await prisma.groupMember.create({ data: { userId, groupId } });
    const token = jwt.sign({ userId, roles: [] }, config.JWT_SECRET);

    const res = await request(app.server).get('/auth/me').set('Authorization', `Bearer ${token}`);

    expect(res.status).toBe(200);
    expect(res.body).toMatchObject({
      userId,
      email: expect.stringContaining('@'),
      roles: [],
      memberships: [
        expect.objectContaining({
          groupId,
          name: expect.any(String),
          membershipRoles: expect.any(Array)
        })
      ]
    });
  });
});
