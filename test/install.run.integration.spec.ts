process.env.SIGNING_SECRET = process.env.SIGNING_SECRET || 'testsecret';

import request from 'supertest';
import jwt from 'jsonwebtoken';
import { buildServer } from '../src/api/server';
import { prisma } from '../src/lib/db';
import { sha256Hex } from '../src/lib/integrity';
import { signHmac } from '../src/lib/signature';
import { config } from '../src/config';
import { setRequestContext } from '../src/lib/request-context';

function token(userId: string, groupId: string) {
  return jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);
}

describe('install/run integration', () => {
  const app = buildServer();
  let groupA: string;
  let groupB: string;
  let userA: string;

  beforeAll(async () => {
    // seed groups/users with request context to satisfy middleware
    groupA = (await prisma.group.create({ data: { name: 'GA', type: 'ORG' } })).id;
    groupB = (await prisma.group.create({ data: { name: 'GB', type: 'ORG' } })).id;
    userA = (await prisma.user.create({ data: { email: 'a@example.com', passwordHash: 'x' } })).id;
  });

  afterAll(async () => {
    await prisma.$disconnect();
  });

  test('stable install rejects non-approved package', async () => {
    const manifest = { name: '@ga/demo', version: '1.0.0', engine: '1.0.0', capabilities: ['echo'] };
    const integrity = sha256Hex(JSON.stringify(manifest));

    // create package with status DRAFT under groupA
    setRequestContext({ groupId: groupA, userId: userA });
    await prisma.componentPackage.create({
      data: {
        name: manifest.name,
        version: manifest.version,
        status: 'DRAFT',
        integrityHash: integrity,
        manifest,
        policyId: (await prisma.componentPolicy.create({ data: { name: 'default' } })).id,
        ownerGroupId: groupA
      }
    });

    const res = await request(app.server)
      .post('/install')
      .set('authorization', 'Bearer ' + token(userA, groupA))
      .send({ name: manifest.name, version: manifest.version, channel: 'STABLE' });

    expect(res.status).toBe(409);
  });

  test('signature mismatch returns 422 on run', async () => {
    const manifest = { name: '@ga/signed', version: '1.0.0', engine: '1.0.0', capabilities: ['echo'] };
    const integrity = sha256Hex(JSON.stringify(manifest));
    const badSignature = signHmac('tampered', config.SIGNING_SECRET || 'testsecret');
    const manifestSigned = { ...manifest, signature: badSignature };

    setRequestContext({ groupId: groupA, userId: userA });
    const policyId = (await prisma.componentPolicy.findFirst())?.id || (await prisma.componentPolicy.create({ data: { name: 'p' } })).id;
    const pkg = await prisma.componentPackage.create({
      data: {
        name: manifest.name,
        version: manifest.version,
        status: 'APPROVED',
        integrityHash: integrity,
        manifest: manifestSigned,
        policyId,
        ownerGroupId: groupA
      }
    });
    setRequestContext({ groupId: groupA, userId: userA });
    const install = await prisma.componentInstall.create({
      data: { packageId: pkg.id, groupId: groupA, lockData: { integrity } }
    });

    const res = await request(app.server)
      .post(`/components/${install.id}/run`)
      .set('authorization', 'Bearer ' + token(userA, groupA))
      .send({ payload: {} });

    expect(res.status).toBe(422);
  });

  test('cross-group run is not found', async () => {
    const manifest = { name: '@ga/echo', version: '1.0.1', engine: '1.0.0', capabilities: ['echo'] };
    const integrity = sha256Hex(JSON.stringify(manifest));
    setRequestContext({ groupId: groupA, userId: userA });
    const policyId = (await prisma.componentPolicy.findFirst())?.id || (await prisma.componentPolicy.create({ data: { name: 'p2' } })).id;
    const pkg = await prisma.componentPackage.create({
      data: { name: manifest.name, version: manifest.version, status: 'APPROVED', integrityHash: integrity, manifest, policyId, ownerGroupId: groupA }
    });
    setRequestContext({ groupId: groupA, userId: userA });
    const install = await prisma.componentInstall.create({ data: { packageId: pkg.id, groupId: groupA, lockData: { integrity } } });

    const res = await request(app.server)
      .post(`/components/${install.id}/run`)
      .set('authorization', 'Bearer ' + token(userA, groupB))
      .send({ payload: { foo: 'bar' } });

    expect(res.status).toBe(404);
  });

  test('unsupported capability returns error but 200', async () => {
    const manifest = { name: '@ga/unknown-cap', version: '1.0.2', engine: '1.0.0', capabilities: ['does.not.exist'] };
    const integrity = sha256Hex(JSON.stringify(manifest));
    setRequestContext({ groupId: groupA, userId: userA });
    const policyId = (await prisma.componentPolicy.findFirst())?.id || (await prisma.componentPolicy.create({ data: { name: 'p3' } })).id;
    const pkg = await prisma.componentPackage.create({
      data: { name: manifest.name, version: manifest.version, status: 'APPROVED', integrityHash: integrity, manifest, policyId, ownerGroupId: groupA }
    });
    setRequestContext({ groupId: groupA, userId: userA });
    const install = await prisma.componentInstall.create({ data: { packageId: pkg.id, groupId: groupA, lockData: { integrity } } });

    const res = await request(app.server)
      .post(`/components/${install.id}/run`)
      .set('authorization', 'Bearer ' + token(userA, groupA))
      .send({ payload: {} });

    expect(res.status).toBe(200);
    expect(res.body.results['does.not.exist'].error).toBe('unsupported_capability');
  });
});
