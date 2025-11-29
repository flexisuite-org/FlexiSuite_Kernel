import request from 'supertest';
import jwt from 'jsonwebtoken';
import { buildServer } from '../src/api/server';
import { prisma } from '../src/lib/db';
import { sha256Hex } from '../src/lib/integrity';
import { config } from '../src/config';
import { setRequestContext } from '../src/lib/request-context';

process.env.SIGNING_SECRET = process.env.SIGNING_SECRET || 'testsecret';

function token(userId: string, groupId: string) {
  return jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);
}

describe('capability allowlist', () => {
  const app = buildServer();
  let groupId: string;
  let userId: string;

  beforeAll(async () => {
    groupId = (await prisma.group.create({ data: { name: 'Gcap', type: 'ORG' } })).id;
    userId = (await prisma.user.create({ data: { email: 'c@c.com', passwordHash: 'x' } })).id;
  });

  afterAll(async () => {
    await prisma.$disconnect();
  });

  it('filters capabilities not in allowlist', async () => {
    const manifest = { name: '@gcap/pkg', version: '1.0.0', engine: '1.0.0', capabilities: ['echo', 'does.not.exist'] };
    const integrity = sha256Hex(JSON.stringify(manifest));
    const policyId = (await prisma.componentPolicy.create({ data: { name: 'pcap' } })).id;
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
