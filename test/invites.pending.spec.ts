import request from 'supertest';
import jwt from 'jsonwebtoken';
import { buildServer } from '../src/api/server';
import { config } from '../src/config';
import { createTenantSeed } from './helpers/seed';
import { prisma } from '../src/lib/db';

describe('GET /invites/pending', () => {
  const app = buildServer();

  beforeAll(async () => {
    await app.ready();
  });

  afterAll(async () => {
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  it('returns pending email-based invites for the current user', async () => {
    const suffix = 'invites';
    const { groupId, userId } = await createTenantSeed(suffix);
    const user = await prisma.user.findUnique({ where: { id: userId } });
    const inviter = await prisma.user.create({ data: { email: `inviter+${suffix}@example.com`, passwordHash: 'x' } });

    await prisma.groupInvite.create({
      data: {
        groupId,
        kind: 'EMAIL',
        email: user?.email ?? '',
        code: 'launcher-invite',
        createdBy: inviter.id,
        expiresAt: new Date(Date.now() + 60 * 1000)
      }
    });

    const token = jwt.sign({ userId, roles: [] }, config.JWT_SECRET);
    const res = await request(app.server).get('/invites/pending').set('Authorization', `Bearer ${token}`);

    expect(res.status).toBe(200);
    expect(res.body).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          groupId,
          inviterUserId: inviter.id,
          inviterEmail: inviter.email,
          groupName: expect.any(String)
        })
      ])
    );
  });
});
