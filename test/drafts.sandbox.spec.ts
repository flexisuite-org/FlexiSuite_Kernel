import request from 'supertest';
import { buildServer } from '../src/api/server';

describe('draft sandbox', () => {
  it('requires auth/context', async () => {
    const app = buildServer();
    const res = await request(app.server).post('/sandbox/drafts/run').send({ script: 'return 1;' });
    expect(res.status).toBe(401);
  });
});
