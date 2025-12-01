import argon2 from 'argon2';
import jwt from 'jsonwebtoken';
import request from 'supertest';
import { buildServer } from '../src/api/server';
import { prisma } from '../src/lib/db';
import { config } from '../src/config';

function tokenFor(userId: string, groupId?: string | null) {
  return jwt.sign({ userId, groupId: groupId ?? null, roles: [] }, config.JWT_SECRET);
}

async function createUser(email: string, password = 'password123') {
  const passwordHash = await argon2.hash(password);
  return prisma.user.create({ data: { email, passwordHash } });
}

describe('auth invites', () => {
  const app = buildServer();

  beforeAll(async () => {
    await app.ready();
  });

  afterAll(async () => {
    await prisma.$disconnect();
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  it('allows signup with a valid account invite', async () => {
    const admin = await createUser('admin@invites.example');
    const inviteEmail = 'guest@invites.example';
    const createRes = await request(app.server)
      .post('/auth/account-invites')
      .set('authorization', `Bearer ${tokenFor(admin.id)}`)
      .send({ email: inviteEmail });
    expect(createRes.status).toBe(201);
    const code = createRes.body.code;
    expect(code).toBeTruthy();

    const signupRes = await request(app.server)
      .post('/auth/signup')
      .send({ email: inviteEmail, password: 'newpass123', accountInviteCode: code });
    expect(signupRes.status).toBe(200);
    expect(signupRes.body.accessToken).toBeDefined();
    expect(signupRes.body.refreshToken).toBeDefined();

    const inviteRecord = await prisma.accountInvite.findUnique({ where: { code } });
    expect(inviteRecord?.usedAt).toBeTruthy();
  });

  it('rejects signup when the invite code is unknown', async () => {
    const res = await request(app.server)
      .post('/auth/signup')
      .send({ email: 'missing@example.com', password: 'password123', accountInviteCode: 'nope' });
    expect(res.status).toBe(404);
  });

  it('rejects signup when the invite has expired', async () => {
    const expiredCode = 'expired-code';
    await prisma.accountInvite.create({
      data: {
        email: 'expired@example.com',
        code: expiredCode,
        expiresAt: new Date(Date.now() - 1000 * 60),
        createdAt: new Date(),
        createdBy: null
      }
    });

    const res = await request(app.server)
      .post('/auth/signup')
      .send({ email: 'expired@example.com', password: 'password123', accountInviteCode: expiredCode });
    expect(res.status).toBe(410);
  });

  it('accepts a LINK group invite and creates a membership', async () => {
    const group = await prisma.group.create({ data: { name: 'link-group', type: 'ORG' } });
    const owner = await createUser('owner@link.example');
    const inviteRes = await request(app.server)
      .post('/invites/group-invites')
      .set('authorization', `Bearer ${tokenFor(owner.id, group.id)}`)
      .send({ groupId: group.id, kind: 'LINK' });
    expect(inviteRes.status).toBe(201);
    const code = inviteRes.body.code;
    expect(code).toBeTruthy();

    const invitee = await createUser('link-eat@invites.example');
    const acceptRes = await request(app.server)
      .post(`/invites/group-invites/${code}/accept`)
      .set('authorization', `Bearer ${tokenFor(invitee.id)}`)
      .send();
    expect(acceptRes.status).toBe(200);
    expect(acceptRes.body.accepted).toBe(true);

    const membership = await prisma.groupMember.findFirst({
      where: { userId: invitee.id, groupId: group.id }
    });
    expect(membership).toBeTruthy();
  });

  it('shows EMAIL invites as pending and removes them after accept', async () => {
    const group = await prisma.group.create({ data: { name: 'email-group', type: 'ORG' } });
    const owner = await createUser('owner@email.example');
    const inviteeEmail = 'invitee@email.example';
    const inviteRes = await request(app.server)
      .post('/invites/group-invites')
      .set('authorization', `Bearer ${tokenFor(owner.id, group.id)}`)
      .send({ groupId: group.id, kind: 'EMAIL', email: inviteeEmail });
    expect(inviteRes.status).toBe(201);
    const code = inviteRes.body.code;
    expect(code).toBeTruthy();

    const invitee = await createUser(inviteeEmail);
    const pendingRes = await request(app.server)
      .get('/invites/group-invites/pending')
      .set('authorization', `Bearer ${tokenFor(invitee.id)}`)
      .query({ email: inviteeEmail });
    expect(pendingRes.status).toBe(200);
    expect(pendingRes.body).toHaveLength(1);
    expect(pendingRes.body[0].code).toBe(code);

    const acceptRes = await request(app.server)
      .post(`/invites/group-invites/${code}/accept`)
      .set('authorization', `Bearer ${tokenFor(invitee.id)}`)
      .send();
    expect(acceptRes.status).toBe(200);

    const afterPending = await request(app.server)
      .get('/invites/group-invites/pending')
      .set('authorization', `Bearer ${tokenFor(invitee.id)}`)
      .query({ email: inviteeEmail });
    expect(afterPending.status).toBe(200);
    expect(afterPending.body).toHaveLength(0);
  });
});
