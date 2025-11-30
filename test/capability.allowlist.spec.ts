import request from 'supertest';
import jwt from 'jsonwebtoken';
import { buildServer } from '../src/api/server';
import { prisma } from '../src/lib/db';
import { hashJson } from '../src/lib/integrity';
import { config } from '../src/config';
import { setRequestContext } from '../src/lib/request-context';
import { truncateAll, createTenantSeed, createPolicy } from './helpers/seed';

process.env.SIGNING_SECRET = process.env.SIGNING_SECRET || 'testsecret';

function token(userId: string, groupId: string) {
  return jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);
}

describe('capability allowlist', () => {
  const app = buildServer();
  let groupId: string;
  let userId: string;
  let policyId: string;

  beforeEach(async () => {
    await app.ready();
    await truncateAll();
    const seed = await createTenantSeed(`cap-${Date.now()}`);
    groupId = seed.groupId;
    userId = seed.userId;
    policyId = await createPolicy(`pcap-${Date.now()}`);
  });

  afterAll(async () => {
    await prisma.$disconnect();
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  it('filters capabilities not in allowlist', async () => {
    const manifest = { name: `@gcap/pkg-${Date.now()}`, version: '1.0.0', engine: '1.0.0', capabilities: ['echo', 'does.not.exist'] };
    const integrity = hashJson(manifest);
    setRequestContext({ groupId, userId });
    const pkg = await prisma.componentPackage.create({ data: { name: manifest.name, version: manifest.version, status: 'APPROVED', integrityHash: integrity, manifest, policyId, ownerGroupId: groupId } });
    const install = await prisma.componentInstall.create({ data: { packageId: pkg.id, groupId, lockData: { integrity } } });

    const res = await request(app.server)
      .post(`/components/${install.id}/run`)
      .set('authorization', 'Bearer ' + token(userId, groupId))
      .send({ payload: {} });

    expect(res.status).toBe(200);
    expect(res.body.results['echo']).toBeDefined();
    expect(res.body.results['does.not.exist']).toBeUndefined();
  });
});
