process.env.SIGNING_SECRET = process.env.SIGNING_SECRET || 'testsecret';

import request from 'supertest';
import { buildServer } from '../src/api/server';
import { prisma } from '../src/lib/db';
import { sha256Hex, hashJson } from '../src/lib/integrity';
import { config } from '../src/config';
import { setRequestContext } from '../src/lib/request-context';
import { truncateAll, createTenantSeed, createPolicy } from './helpers/seed';
import jwt from 'jsonwebtoken';

function token(userId: string, groupId: string) {
  return jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);
}

describe('bundle upload', () => {
  const app = buildServer();
  let groupId: string;
  let userId: string;
  let pkgId: string;
  let policyId: string;

  beforeEach(async () => {
    await app.ready();
    await truncateAll();
    const seed = await createTenantSeed(`bundle-${Date.now()}`);
    groupId = seed.groupId;
    userId = seed.userId;
    policyId = await createPolicy(`pb-${Date.now()}`);
    const manifest = { name: `@gup/pkg-${Date.now()}`, version: '1.0.0', engine: '1.0.0', capabilities: [] };
    const integrity = hashJson(manifest);
    setRequestContext({ groupId, userId });
    pkgId = (await prisma.componentPackage.create({ data: { name: manifest.name, version: manifest.version, status: 'DRAFT', integrityHash: integrity, manifest, policyId, ownerGroupId: groupId, createdById: userId } })).id;
  });

  afterAll(async () => {
    await prisma.$disconnect();
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  it('uploads bundle, verifies integrity, sets signature', async () => {
    const buf = Buffer.from('hello bundle');
    const integrity = sha256Hex(buf);
    const res = await request(app.server)
      .post(`/registry/packages/${pkgId}/bundleUpload`)
      .set('authorization', 'Bearer ' + token(userId, groupId))
      .send({ data: buf.toString('base64'), integrity });

    expect(res.status).toBe(201);
    expect(res.body.bundleIntegrity).toBe(integrity);
    expect(res.body.bundleSignature).toBeTruthy();
  });
});
