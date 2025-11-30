import request from 'supertest';
import { buildServer } from '../src/api/server';

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
