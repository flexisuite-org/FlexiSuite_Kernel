import request from 'supertest';
import { buildServer } from '../src/api/server';

describe('components run (api mode)', () => {
  it('returns unauthorized without group context', async () => {
    const app = buildServer();
    await app.ready();
    const res = await request(app.server).post('/components/abc/run').send({});
    expect(res.status).toBe(401);
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });
});
