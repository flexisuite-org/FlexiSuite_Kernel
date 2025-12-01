import request from 'supertest';
import jwt from 'jsonwebtoken';
import { buildServer } from '../src/api/server';
import { prisma } from '../src/lib/db';
import { config } from '../src/config';

const app = buildServer();

function kernelAdminToken(userId: string) {
  return jwt.sign({ userId, groupId: null, roles: ['kernel-admin'] }, config.JWT_SECRET);
}

describe('admin group APIs', () => {
  let authHeader: string;

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
        email: `admin+${Date.now()}@kernel.test`,
        passwordHash: 'x'
      }
    });
    authHeader = 'Bearer ' + kernelAdminToken(admin.id);
  });

  test('POST /admin/groups creates a group and GET /admin/groups lists it', async () => {
    const createRes = await request(app.server)
      .post('/admin/groups')
      .set('authorization', authHeader)
      .send({ name: 'Kernel Tenants', type: 'ORG' });

    expect(createRes.status).toBe(201);
    expect(createRes.body.name).toBe('Kernel Tenants');
    expect(createRes.body.type).toBe('ORG');

    const listRes = await request(app.server)
      .get('/admin/groups')
      .set('authorization', authHeader)
      .query({ name: 'Kernel Tenants' });

    expect(listRes.status).toBe(200);
    expect(Array.isArray(listRes.body.items)).toBe(true);
    expect(listRes.body.items[0].name).toBe('Kernel Tenants');
  });

  test('POST /admin/groups/:id/deactivate flags the group in settings', async () => {
    const createRes = await request(app.server)
      .post('/admin/groups')
      .set('authorization', authHeader)
      .send({ name: 'Pause Tenant', type: 'ORG' });

    expect(createRes.status).toBe(201);
    const groupId = createRes.body.id;

    const deactivateRes = await request(app.server)
      .post(`/admin/groups/${groupId}/deactivate`)
      .set('authorization', authHeader)
      .send();

    expect(deactivateRes.status).toBe(200);
    expect(deactivateRes.body.settings?.adminDisabled).toBe(true);
  });
});
