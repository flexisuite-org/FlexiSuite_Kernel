import request from 'supertest';
import crypto from 'crypto';
import { buildServer } from '../src/api/server';
import { prisma } from '../src/lib/db';
import { truncateAll } from './helpers/seed';

describe('github webhook signature', () => {
  const app = buildServer();

  beforeEach(async () => {
    await app.ready();
    await truncateAll();
  });

  afterAll(async () => {
    await prisma.$disconnect();
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  function signedBody(payload: any, secret: string) {
    const raw = JSON.stringify(payload);
    const sig = 'sha256=' + crypto.createHmac('sha256', secret).update(raw).digest('hex');
    return { raw, sig };
  }

  it('rejects missing or invalid signature when secret is set', async () => {
    const secret = process.env.GITHUB_WEBHOOK_SECRET || 'testhooksecret';
    const payload = { ref: 'refs/heads/main', repository: { full_name: 'demo/repo' } };
    const { raw } = signedBody(payload, secret);

    const res = await request(app.server)
      .post('/integrations/github/webhook')
      .set('content-type', 'application/json')
      .send(raw);

    expect(res.status).toBe(401);
  });

  it('accepts valid signature', async () => {
    const secret = process.env.GITHUB_WEBHOOK_SECRET || 'testhooksecret';
    const payload = { ref: 'refs/heads/main', repository: { full_name: 'demo/repo' }, head_commit: { id: 'abc' } };
    const { raw, sig } = signedBody(payload, secret);

    const res = await request(app.server)
      .post('/integrations/github/webhook')
      .set('content-type', 'application/json')
      .set('x-hub-signature-256', sig)
      .send(raw);

    expect(res.status).toBe(202);
  });
});
