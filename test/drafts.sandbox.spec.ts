import request from 'supertest';
import { buildServer } from '../src/api/server';
import { Prisma } from '@prisma/client';
import { prisma, setRlsContext } from '../src/lib/db';
import { setRequestContext } from '../src/lib/request-context';
import { createPolicy, createTenantSeed } from './helpers/seed';

describe('draft sandbox', () => {
  const app = buildServer();
  afterAll(async () => {
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  beforeAll(async () => {
    await app.ready();
  });

  it('requires auth/context', async () => {
    const res = await request(app.server).post('/sandbox/drafts/run').send({ script: 'return 1;' });
    expect(res.status).toBe(401);
  });
});

describe('draft mode guards', () => {
  it('enables default_transaction_read_only for draft contexts', async () => {
    const { groupId, userId } = await createTenantSeed('draft-readonly');
    const rows = await prisma.$transaction(async (tx) => {
      await tx.$executeRawUnsafe(
        "SELECT set_config('flexi.current_group', $1, true), set_config('flexi.current_user', $2, true), set_config('default_transaction_read_only', $3, true)",
        groupId,
        userId,
        'on'
      );
      return tx.$queryRaw<{ value: string }[]>(Prisma.sql`SELECT current_setting('default_transaction_read_only') AS value`);
    });

    expect(rows[0]?.value).toBe('on');
  });

  it('blocks production writes but allows PlaygroundLog in draft', async () => {
    const { groupId, userId } = await createTenantSeed('draft-write');
    setRequestContext({ groupId, userId, mode: 'stable' });
    await setRlsContext(groupId, userId, 'stable');

    const policyId = await createPolicy('draft-write-policy');
    const componentPackage = await prisma.componentPackage.create({
      data: {
        name: 'draft-write-package',
        version: '1.0.0',
        status: 'APPROVED',
        integrityHash: 'draft-write',
        manifest: { test: 'draft' },
        policyId,
        ownerGroupId: groupId,
        createdById: userId
      }
    });

    const app = await prisma.app.create({ data: { name: 'draft-write-app', version: '1.0.0' } });
    const definition = await prisma.entityDefinition.create({
      data: {
        appId: app.id,
        name: 'draft-def',
        version: 1,
        schema: { type: 'object', properties: {} },
        strict: true
      }
    });

    setRequestContext({ groupId, userId, mode: 'draft' });
    await setRlsContext(groupId, userId, 'draft');

    await expect(
      prisma.entityRecord.create({
        data: {
          definitionId: definition.id,
          groupId,
          data: { foo: 'bar' },
          schemaVersion: 1
        }
      })
    ).rejects.toThrow('write_not_allowed_in_draft');

    await expect(
      prisma.componentInstall.create({
        data: {
          packageId: componentPackage.id,
          groupId,
          lockData: { version: '1.0.0' },
          channel: 'STABLE',
          installedBy: userId
        }
      })
    ).rejects.toThrow('write_not_allowed_in_draft');

    const log = await prisma.playgroundLog.create({
      data: {
        groupId,
        payload: { allowed: true }
      }
    });

    expect(log.groupId).toBe(groupId);
  });
});
