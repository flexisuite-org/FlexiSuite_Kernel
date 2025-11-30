import request from 'supertest';
import jwt from 'jsonwebtoken';
import { buildServer } from '../src/api/server';
import { config } from '../src/config';
import { createTenantSeed } from './helpers/seed';
import { prisma } from '../src/lib/db';

describe('sandbox clone entities routes', () => {
  const app = buildServer();

  beforeAll(async () => {
    await app.ready();
  });

  afterAll(async () => {
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  it('returns placeholder stats for each requested spec', async () => {
    const { groupId, userId } = await createTenantSeed('clone-success');
    const sandboxGroup = await prisma.group.create({ data: { name: 'sandbox-clone', type: 'ORG' } });
    const session = await prisma.sandboxSession.create({
      data: {
        sourceGroupId: groupId,
        sandboxGroupId: sandboxGroup.id,
        appId: 'clone-test',
        expiresAt: new Date(Date.now() + 60 * 60 * 1000)
      }
    });

    const token = jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);
    const specs = [
      { model: 'EntityRecord', ids: ['a', 'b'] },
      { model: 'AppInstall', whereJson: { foo: 'bar' } }
    ];

    const res = await request(app.server)
      .post(`/sandbox/sessions/${session.id}/clone-entities`)
      .set('Authorization', `Bearer ${token}`)
      .send({ specs });

    expect(res.status).toBe(200);
    expect(res.body.results).toHaveLength(specs.length);
    expect(res.body.results[0]).toMatchObject({ model: 'EntityRecord', requested: 2, cloned: 0, skipped: 2 });
    expect(res.body.results[1]).toMatchObject({ model: 'AppInstall', cloned: 0, skipped: 0 });
  });

  it('rejects clone request for a missing sandbox session', async () => {
    const { groupId, userId } = await createTenantSeed('clone-missing');
    const token = jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);

    const res = await request(app.server)
      .post(`/sandbox/sessions/does-not-exist/clone-entities`)
      .set('Authorization', `Bearer ${token}`)
      .send({ specs: [{ model: 'EntityRecord' }] });

    expect(res.status).toBe(404);
    expect(res.body).toEqual({ error: 'sandbox_session_not_found' });
  });

  it('returns 400 when the body does not include specs', async () => {
    const { groupId, userId } = await createTenantSeed('clone-no-specs');
    const sandboxGroup = await prisma.group.create({ data: { name: 'sandbox-no-specs', type: 'ORG' } });
    const session = await prisma.sandboxSession.create({
      data: {
        sourceGroupId: groupId,
        sandboxGroupId: sandboxGroup.id,
        appId: 'clone-test',
        expiresAt: new Date(Date.now() + 60 * 60 * 1000)
      }
    });
    const token = jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);

    const res = await request(app.server)
      .post(`/sandbox/sessions/${session.id}/clone-entities`)
      .set('Authorization', `Bearer ${token}`)
      .send({});

    expect(res.status).toBe(400);
    expect(res.body.error).toBe('invalid_input');
  });
});
