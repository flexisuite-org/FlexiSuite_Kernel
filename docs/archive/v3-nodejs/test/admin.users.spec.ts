import request from 'supertest';
import jwt from 'jsonwebtoken';
import { buildServer } from '../src/api/server';
import { prisma } from '../src/lib/db';
import { config } from '../src/config';
import { setRequestContext } from '../src/lib/request-context';

const app = buildServer();

function kernelAdminToken(userId: string) {
  return jwt.sign({ userId, groupId: null, roles: ['kernel-admin'] }, config.JWT_SECRET);
}

describe('admin user APIs', () => {
  let authHeader: string;
  let targetUserId: string;
  let targetGroupId: string;

  beforeAll(async () => {
    await app.ready();
  });

  afterAll(async () => {
    await app.close();
    await prisma.$disconnect();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis().catch(() => {});
  });

  beforeEach(async () => {
    const admin = await prisma.user.create({
      data: {
        email: `kernel-admin+${Date.now()}@example.com`,
        passwordHash: 'x'
      }
    });
    authHeader = 'Bearer ' + kernelAdminToken(admin.id);

    const target = await prisma.user.create({
      data: {
        email: `user+${Date.now()}@example.com`,
        passwordHash: 'x'
      }
    });
    targetUserId = target.id;

    const group = await prisma.group.create({ data: { name: 'Tenant', type: 'ORG' } });
    targetGroupId = group.id;

    setRequestContext({ groupId: group.id, userId: admin.id });
    await prisma.groupMember.create({ data: { userId: targetUserId, groupId: group.id } });
    setRequestContext({ groupId: null, userId: null });

    await prisma.refreshToken.create({
      data: {
        userId: targetUserId,
        tokenHash: 'token',
        familyId: 'family',
        expiresAt: new Date(Date.now() + 100000)
      }
    });
  });

  test('GET /admin/users supports filtering by group', async () => {
    const res = await request(app.server)
      .get('/admin/users')
      .set('authorization', authHeader)
      .query({ groupId: targetGroupId });

    expect(res.status).toBe(200);
    expect(Array.isArray(res.body.items)).toBe(true);
    expect(res.body.items.some((item: any) => item.id === targetUserId)).toBe(true);
  });

  test('GET /admin/users/:id returns memberships', async () => {
    const res = await request(app.server)
      .get(`/admin/users/${targetUserId}`)
      .set('authorization', authHeader);

    expect(res.status).toBe(200);
    expect(res.body.id).toBe(targetUserId);
    const membership = res.body.memberships[0];
    expect(membership.group.id).toBe(targetGroupId);
  });

  test('POST /admin/users/:id/force-logout revokes refresh tokens', async () => {
    const existing = await prisma.refreshToken.findMany({ where: { userId: targetUserId } });
    expect(existing.length).toBeGreaterThan(0);
    expect(existing.some((token) => !token.revoked)).toBe(true);

    const res = await request(app.server)
      .post(`/admin/users/${targetUserId}/force-logout`)
      .set('authorization', authHeader);

    expect(res.status).toBe(200);
    expect(res.body.revoked).toBeGreaterThan(0);

    const after = await prisma.refreshToken.findMany({ where: { userId: targetUserId } });
    expect(after.every((token) => token.revoked)).toBe(true);
  });
});
