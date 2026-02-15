process.env.SIGNING_SECRET = process.env.SIGNING_SECRET || 'testsecret';

import request from 'supertest';
import jwt from 'jsonwebtoken';
import { buildServer } from '../src/api/server';
import { prisma } from '../src/lib/db';
import { hashJson } from '../src/lib/integrity';
import { signHmac } from '../src/lib/signature';
import { config } from '../src/config';
import { setRequestContext } from '../src/lib/request-context';
import { truncateAll, createTenantSeed, createPolicy } from './helpers/seed';

function token(userId: string, groupId: string) {
  return jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);
}

describe('install/run integration', () => {
  const app = buildServer();
  let groupA: string;
  let groupB: string;
  let userA: string;
  let policyId: string;

  beforeEach(async () => {
    jest.setTimeout(20000);
    await app.ready();
    await truncateAll();
    const a = await createTenantSeed(`a-${Date.now()}`);
    const b = await createTenantSeed(`b-${Date.now()}`);
    groupA = a.groupId;
    userA = a.userId;
    groupB = b.groupId;
    policyId = await createPolicy(`p-${Date.now()}`);
  });

  afterAll(async () => {
    await prisma.$disconnect();
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  test('stable install rejects non-approved package', async () => {
    const manifest = { name: `@ga/demo-${Date.now()}`, version: '1.0.0', engine: '1.0.0', capabilities: ['echo'] };
    const integrity = hashJson(manifest);

    // create package with status DRAFT under groupA
    setRequestContext({ groupId: groupA, userId: userA });
    const pkg = await prisma.componentPackage.create({
      data: {
        name: manifest.name,
        version: manifest.version,
        status: 'DRAFT',
        integrityHash: integrity,
        manifest,
        policyId,
        ownerGroupId: groupA
      }
    });

    await prisma.componentInstall.create({
      data: {
        packageId: pkg.id,
        groupId: groupA,
        channel: 'DRAFT',
        lockData: { integrity: integrity }
      }
    });

    const res = await request(app.server)
      .post('/install')
      .set('authorization', 'Bearer ' + token(userA, groupA))
      .send({ name: manifest.name, version: manifest.version, channel: 'STABLE' });

    expect(res.status).toBe(409);
  });

  test('signature mismatch returns 422 on run', async () => {
    const manifest = { name: `@ga/signed-${Date.now()}`, version: '1.0.0', engine: '1.0.0', capabilities: ['echo'] };
    const integrity = hashJson(manifest);
    const badSignature = signHmac('tampered', config.SIGNING_SECRET || 'testsecret');
    const manifestSigned = { ...manifest, signature: badSignature };

    setRequestContext({ groupId: groupA, userId: userA });
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
    const install = await prisma.componentInstall.create({
      data: { packageId: pkg.id, groupId: groupA, lockData: { integrity } }
    });

    const res = await request(app.server)
      .post(`/components/${install.id}/run`)
      .set('authorization', 'Bearer ' + token(userA, groupA))
      .send({ payload: {} });

    expect(res.status).toBe(422);
  });

  test('bundle signature mismatch blocks install', async () => {
    const manifest = { name: `@ga/bundle-${Date.now()}`, version: '1.0.5', engine: '1.0.0', capabilities: ['echo'] };
    const integrity = hashJson(manifest);
    setRequestContext({ groupId: groupA, userId: userA });
    const pkg = await prisma.componentPackage.create({
      data: {
        name: manifest.name,
        version: manifest.version,
        status: 'APPROVED',
        integrityHash: integrity,
        bundleIntegrity: 'deadbeef',
        bundleSignature: 'wrongsig',
        manifest,
        policyId,
        ownerGroupId: groupA
      }
    });
    await prisma.componentInstall.create({ data: { packageId: pkg.id, groupId: groupA, lockData: { integrity } } });

    const res = await request(app.server)
      .post('/install')
      .set('authorization', 'Bearer ' + token(userA, groupA))
      .send({ name: manifest.name, version: manifest.version, channel: 'STABLE' });

    expect(res.status).toBeGreaterThanOrEqual(400);
  });

  test('cross-group run is not found', async () => {
    const manifest = { name: `@ga/echo-${Date.now()}`, version: '1.0.1', engine: '1.0.0', capabilities: ['echo'] };
    const integrity = hashJson(manifest);
    setRequestContext({ groupId: groupA, userId: userA });
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
    const manifest = { name: `@ga/unknown-cap-${Date.now()}`, version: '1.0.2', engine: '1.0.0', capabilities: ['does.not.exist'] };
    const integrity = hashJson(manifest);
    setRequestContext({ groupId: groupA, userId: userA });
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

  test('entity.list respects group isolation', async () => {
    const manifest = { name: `@ga/list-${Date.now()}`, version: '1.0.3', engine: '1.0.0', capabilities: ['data.entity.list'] };
    const integrity = hashJson(manifest);
    setRequestContext({ groupId: groupA, userId: userA });
    const pkg = await prisma.componentPackage.create({
      data: { name: manifest.name, version: manifest.version, status: 'APPROVED', integrityHash: integrity, manifest, policyId, ownerGroupId: groupA }
    });
    setRequestContext({ groupId: groupA, userId: userA });
    const install = await prisma.componentInstall.create({ data: { packageId: pkg.id, groupId: groupA, lockData: { integrity } } });

    // seed records in groupA
    const def = await prisma.entityDefinition.create({ data: { appId: (await prisma.app.create({ data: { name: 'a', version: '1' } })).id, name: 'n', version: 1, schema: {}, strict: false } });
    await prisma.entityRecord.create({ data: { definitionId: def.id, groupId: groupA, data: { foo: 'bar' }, schemaVersion: 1 } });

    // request as groupB should return empty due to RLS/middleware scoping
    const res = await request(app.server)
      .post(`/components/${install.id}/run`)
      .set('authorization', 'Bearer ' + token(userA, groupB))
      .send({ payload: { limit: 10 } });

    expect(res.status).toBe(404); // install not found in groupB
  });

  test('listByDefinition and getDefinition work in own group', async () => {
    const manifest = { name: `@ga/list-def-${Date.now()}`, version: '1.0.6', engine: '1.0.0', capabilities: ['data.entity.listByDefinition', 'data.entity.getDefinition'] };
    const integrity = hashJson(manifest);
    setRequestContext({ groupId: groupA, userId: userA });
    const pkg = await prisma.componentPackage.create({ data: { name: manifest.name, version: manifest.version, status: 'APPROVED', integrityHash: integrity, manifest, policyId, ownerGroupId: groupA } });
    const install = await prisma.componentInstall.create({ data: { packageId: pkg.id, groupId: groupA, lockData: { integrity } } });

    const def = await prisma.entityDefinition.create({ data: { appId: (await prisma.app.create({ data: { name: 'a3', version: '1' } })).id, name: 'defx', version: 1, schema: {}, strict: false } });

    const res = await request(app.server)
      .post(`/components/${install.id}/run`)
      .set('authorization', 'Bearer ' + token(userA, groupA))
      .send({ payload: { definitionId: def.id, limit: 10 } });

    expect(res.status).toBe(200);
    expect(res.body.results['data.entity.getDefinition'].id).toBe(def.id);
  });
});
