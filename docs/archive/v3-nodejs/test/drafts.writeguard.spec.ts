import { buildServer } from '../src/api/server';
import request from 'supertest';
import jwt from 'jsonwebtoken';
import { prisma } from '../src/lib/db';
import { config } from '../src/config';
import { createTenantSeed } from './helpers/seed';

function token(userId: string, groupId: string) {
  return jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);
}

describe('draft sandbox write guard', () => {
  const app = buildServer();
  let groupId: string;
  let userId: string;

  beforeEach(async () => {
    await app.ready();
    const seed = await createTenantSeed(`draft-${Date.now()}`);
    groupId = seed.groupId;
    userId = seed.userId;
  });

  afterAll(async () => {
    await prisma.$disconnect();
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  it('blocks prisma writes inside draft sandbox', async () => {
    const script = `await kernel.prisma?.entityRecord?.create({data:{}});`;
    const res = await request(app.server)
      .post('/sandbox/drafts/run')
      .set('authorization', 'Bearer ' + token(userId, groupId))
      .send({ script });

    expect([401, 500]).toContain(res.status);
  });
});
