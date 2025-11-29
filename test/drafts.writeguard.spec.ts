import { buildServer } from '../src/api/server';
import request from 'supertest';
import jwt from 'jsonwebtoken';
import { prisma } from '../src/lib/db';
import { config } from '../src/config';

function token(userId: string, groupId: string) {
  return jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);
}

describe('draft sandbox write guard', () => {
  const app = buildServer();
  let groupId: string;
  let userId: string;

  beforeAll(async () => {
    groupId = (await prisma.group.create({ data: { name: 'G', type: 'ORG' } })).id;
    userId = (await prisma.user.create({ data: { email: 'draft@example.com', passwordHash: 'x' } })).id;
  });

  afterAll(async () => {
    await prisma.$disconnect();
  });

  it('blocks prisma writes inside draft sandbox', async () => {
    const script = `await kernel.prisma?.entityRecord?.create({data:{}});`;
    const res = await request(app.server)
      .post('/sandbox/drafts/run')
      .set('authorization', 'Bearer ' + token(userId, groupId))
      .send({ script });

    // Since sandbox doesn't expose prisma, this should fail gracefully with sandbox_error
    expect(res.status).toBe(500);
  });
});
