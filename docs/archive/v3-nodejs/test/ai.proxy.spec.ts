import request from 'supertest';
import jwt from 'jsonwebtoken';
import { buildServer } from '../src/api/server';
import { aiRateLimiter } from '../src/lib/ai';
import { config } from '../src/config';
import { createTenantSeed } from './helpers/seed';
import { prisma } from '../src/lib/db';

const app = buildServer();

function token(userId: string, groupId: string) {
  return jwt.sign({ userId, groupId, roles: [] }, config.JWT_SECRET);
}

describe('AI proxy /ai/chat', () => {
  beforeAll(async () => {
    await app.ready();
  });

  beforeEach(() => {
    aiRateLimiter.reset();
  });

  afterAll(async () => {
    await prisma.$disconnect();
    await app.close();
    const { closeRedis } = await import('../src/lib/redis');
    await closeRedis();
  });

  it('rejects when groupId is missing', async () => {
    const res = await request(app.server).post('/ai/chat').send({ messages: [{ role: 'user', content: 'hi' }] });
    expect(res.status).toBe(401);
  });

  it('proxies to OpenAI using provided apiKey and logs usage', async () => {
    const { groupId, userId } = await createTenantSeed(`ai-${Date.now()}`);
    const fetchMock = jest.spyOn(global, 'fetch').mockResolvedValue(
      new global.Response(
        JSON.stringify({
          id: 'chatcmpl-test',
          choices: [
            { index: 0, message: { role: 'assistant', content: 'hello!' }, finish_reason: 'stop' }
          ],
          usage: { prompt_tokens: 3, completion_tokens: 5, total_tokens: 8 },
          model: 'gpt-4o-mini'
        }),
        { status: 200, headers: { 'Content-Type': 'application/json' } }
      ) as any
    );

    const res = await request(app.server)
      .post('/ai/chat')
      .set('authorization', 'Bearer ' + token(userId, groupId))
      .send({ messages: [{ role: 'user', content: 'hello' }], apiKey: 'user-openai-key' });

    expect(res.status).toBe(200);
    expect(res.body.choices[0].message.content).toBe('hello!');
    expect(fetchMock).toHaveBeenCalled();

    const logs = await prisma.auditLog.findMany({ where: { groupId, resource: 'ai.chat' } });
    expect(logs.length).toBe(1);
    expect((logs[0].metadata as any)?.provider).toBe('openai');

    fetchMock.mockRestore();
  });

  it('infers Gemini from model name and uses override key', async () => {
    const { groupId, userId } = await createTenantSeed(`gem-${Date.now()}`);
    const fetchMock = jest.spyOn(global, 'fetch').mockImplementation((input: any) => {
      expect(String(input)).toContain('gemini');
      return Promise.resolve(
        new global.Response(
          JSON.stringify({
            candidates: [{ content: { parts: [{ text: 'gemini response' }] } }],
            usageMetadata: { promptTokenCount: 2, candidatesTokenCount: 3, totalTokenCount: 5 },
            model: 'models/gemini-1.5-flash'
          }),
          { status: 200, headers: { 'Content-Type': 'application/json' } }
        ) as any
      );
    });

    const res = await request(app.server)
      .post('/ai/chat')
      .set('authorization', 'Bearer ' + token(userId, groupId))
      .send({ model: 'gemini-1.5-flash', messages: [{ role: 'user', content: 'hola' }], apiKey: 'gem-key' });

    expect(res.status).toBe(200);
    expect(res.body.choices[0].message.content).toBe('gemini response');
    expect(res.body.provider).toBe('gemini');
    fetchMock.mockRestore();
  });

  it('enforces per-group and per-user rate limits', async () => {
    const { groupId, userId } = await createTenantSeed(`limit-${Date.now()}`);
    aiRateLimiter.seed(`grp:${groupId}`, config.aiRateLimit.max);
    aiRateLimiter.seed(`grp:${groupId}:user:${userId}`, config.aiRateLimit.max);

    const res = await request(app.server)
      .post('/ai/chat')
      .set('authorization', 'Bearer ' + token(userId, groupId))
      .send({ messages: [{ role: 'user', content: 'limit' }], apiKey: 'user-openai-key' });

    expect(res.status).toBe(429);
  });
});
