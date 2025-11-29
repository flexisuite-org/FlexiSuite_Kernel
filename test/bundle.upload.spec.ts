import request from 'supertest';
import { buildServer } from '../src/api/server';
import { prisma } from '../src/lib/db';
import { sha256Hex } from '../src/lib/integrity';
import { config } from '../src/config';
import jwt from 'jsonwebtoken';

process.env.SIGNING_SECRET = process.env.SIGNING_SECRET || 'testsecret';

function token(userId: string, groupId: string) {
  return jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);
}

describe('bundle upload', () => {
  const app = buildServer();
  let groupId: string;
  let userId: string;
  let pkgId: string;

  beforeAll(async () => {
    groupId = (await prisma.group.create({ data: { name: 'GUP', type: 'ORG' } })).id;
    userId = (await prisma.user.create({ data: { email: 'u@u.com', passwordHash: 'x' } })).id;
    const manifest = { name: '@gup/pkg', version: '1.0.0', engine: '1.0.0', capabilities: [] };
    const integrity = sha256Hex(JSON.stringify(manifest));
    const policyId = (await prisma.componentPolicy.create({ data: { name: 'pb' } })).id;
    pkgId = (await prisma.componentPackage.create({ data: { name: manifest.name, version: manifest.version, status: 'DRAFT', integrityHash: integrity, manifest, policyId, ownerGroupId: groupId, createdById: userId } })).id;
  });

  afterAll(async () => {
    await prisma.$disconnect();
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
