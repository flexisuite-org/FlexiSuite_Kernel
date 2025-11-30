process.env.CAPABILITY_ROLE_ALLOWLIST = process.env.CAPABILITY_ROLE_ALLOWLIST || JSON.stringify({ echo: ['admin'] });

import request from 'supertest';
import jwt from 'jsonwebtoken';
import { prisma } from '../src/lib/db';
import { hashJson } from '../src/lib/integrity';
import { setRequestContext } from '../src/lib/request-context';
import { truncateAll, createTenantSeed, createPolicy } from './helpers/seed';

function token(userId: string, groupId: string, roles: string[]) {
  const { config } = require('../src/config');
  return jwt.sign({ userId, groupId, roles }, config.JWT_SECRET);
}

describe('capability role allowlist', () => {
  let buildServer: any;

  beforeAll(() => {
    jest.resetModules();
    buildServer = require('../src/api/server').buildServer;
  });

  afterAll(async () => {
    jest.resetModules();
    await prisma.$disconnect();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  it('blocks capability when required role missing, allows when present', async () => {
    const app = buildServer();
    await app.ready();
    await truncateAll();
    const seed = await createTenantSeed(`role-${Date.now()}`);
    const groupId = seed.groupId;
    const userId = seed.userId;
    const policyId = await createPolicy(`pol-${Date.now()}`);

    const manifest = { name: `@role/echo-${Date.now()}`, version: '1.0.0', engine: '1.0.0', capabilities: ['echo'] };
    const integrity = hashJson(manifest);
    setRequestContext({ groupId, userId });
    const pkg = await prisma.componentPackage.create({
      data: { name: manifest.name, version: manifest.version, status: 'APPROVED', integrityHash: integrity, manifest, policyId, ownerGroupId: groupId }
    });
    const install = await prisma.componentInstall.create({ data: { packageId: pkg.id, groupId, lockData: { integrity } } });

    const resNoRole = await request(app.server)
      .post(`/components/${install.id}/run`)
      .set('authorization', 'Bearer ' + token(userId, groupId, []))
      .send({ payload: {} });
    expect(resNoRole.status).toBe(200);
    expect(resNoRole.body.results['echo'].error).toBe('forbidden');

    const resWithRole = await request(app.server)
      .post(`/components/${install.id}/run`)
      .set('authorization', 'Bearer ' + token(userId, groupId, ['admin']))
      .send({ payload: {} });
    expect(resWithRole.status).toBe(200);
    expect(resWithRole.body.results['echo'].error).toBeUndefined();

    await app.close();
  });
});
