import request from 'supertest';
import jwt from 'jsonwebtoken';
import { buildServer } from '../src/api/server';
import { config } from '../src/config';
import { prisma } from '../src/lib/db';
import { setRequestContext } from '../src/lib/request-context';
import { createPackage, createTenantSeed } from './helpers/seed';

describe('GET /launcher/groups', () => {
  const app = buildServer();

  beforeAll(async () => {
    await app.ready();
  });

  afterAll(async () => {
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  it('summarizes installs per membership', async () => {
    const { groupId, userId } = await createTenantSeed('launcher');
    await prisma.groupMember.create({ data: { userId, groupId } });
    const secondGroup = await prisma.group.create({ data: { name: 'Second', type: 'TEAM' } });
    await prisma.groupMember.create({ data: { userId, groupId: secondGroup.id } });

    const pkgA = await createPackage({ name: 'core-app', version: '1.0.0', groupId, userId, status: 'APPROVED' });
    const pkgB = await createPackage({
      name: 'toolkit',
      version: '0.1.0',
      groupId: secondGroup.id,
      userId,
      status: 'APPROVED'
    });

    setRequestContext({ groupId, userId });
    await prisma.componentInstall.create({
      data: {
        packageId: pkgA.id,
        groupId,
        channel: 'STABLE',
        lockData: {}
      }
    });

    setRequestContext({ groupId: secondGroup.id, userId });
    await prisma.componentInstall.create({
      data: {
        packageId: pkgB.id,
        groupId: secondGroup.id,
        channel: 'STABLE',
        lockData: {}
      }
    });

    const token = jwt.sign({ userId, roles: [] }, config.JWT_SECRET);
    const res = await request(app.server).get('/launcher/groups').set('Authorization', `Bearer ${token}`);

    expect(res.status).toBe(200);
    expect(res.body).toHaveLength(2);
    expect(res.body).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          groupId,
          installs: expect.arrayContaining([expect.objectContaining({ packageId: pkgA.id })])
        }),
        expect.objectContaining({
          groupId: secondGroup.id,
          installs: expect.arrayContaining([expect.objectContaining({ packageId: pkgB.id })])
        })
      ])
    );
  });
});
