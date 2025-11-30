import { AddressInfo } from 'net';
import jwt from 'jsonwebtoken';
import { randomUUID } from 'crypto';
import { buildServer } from '../src/api/server';
import { config } from '../src/config';
import { publishWs } from '../src/lib/ws-bus';
import { prisma } from '../src/lib/db';
import { closeRedis } from '../src/lib/redis';

const WebSocketImpl = (globalThis as any).WebSocket as any;

function token(userId: string, groupId: string, roles: string[] = []) {
  return jwt.sign({ userId, groupId, roles }, config.JWT_SECRET);
}

function waitForClose(ws: any, timeout = 1500) {
  return new Promise<any>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('close timeout')), timeout);
    ws.addEventListener('close', (evt: any) => {
      clearTimeout(timer);
      resolve(evt);
    });
    ws.addEventListener('error', (err: any) => {
      clearTimeout(timer);
      reject(err);
    });
  });
}

function waitForMessage(ws: any, matcher: (data: any) => boolean, timeout = 2000) {
  return new Promise<any>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('message timeout')), timeout);
    ws.addEventListener('message', (evt: any) => {
      try {
        const raw = typeof evt.data === 'string' ? evt.data : Buffer.from(evt.data).toString();
        const parsed = JSON.parse(raw);
        if (matcher(parsed)) {
          clearTimeout(timer);
          resolve(parsed);
        }
      } catch (err) {
        clearTimeout(timer);
        reject(err);
      }
    });
    ws.addEventListener('close', (evt: any) => {
      clearTimeout(timer);
      reject(new Error(`socket closed early: ${evt.code}`));
    });
  });
}

async function waitForReady(ws: any) {
  return waitForMessage(ws, (msg) => msg.type === 'ready');
}

describe('websocket channels', () => {
  const app = buildServer();
  let baseUrl: string;

  beforeAll(async () => {
    await app.listen({ port: 0 });
    const address = app.server.address() as AddressInfo;
    baseUrl = `ws://127.0.0.1:${address.port}/ws`;
  });

  afterAll(async () => {
    await app.close();
    await prisma.$disconnect().catch(() => {});
    await closeRedis().catch(() => {});
  });

  it('rejects connections without JWT', async () => {
    const ws = new WebSocketImpl(baseUrl);
    const closeEvt = await waitForClose(ws);
    expect(closeEvt.code).toBe(1008);
  });

  it('delivers published message to subscribed channel', async () => {
    const groupId = randomUUID();
    const userId = randomUUID();
    const channel = `job:${randomUUID()}`;
    const ws = new WebSocketImpl(baseUrl, [token(userId, groupId)]);
    await waitForReady(ws);

    ws.send(JSON.stringify({ type: 'subscribe', channel }));
    await waitForMessage(ws, (msg) => msg.type === 'subscribed' && msg.channel === channel);

    const payload = { status: 'running', message: 'starting', step: 1 };
    await publishWs(channel, payload, { groupId });

    const received = await waitForMessage(
      ws,
      (msg) => msg.channel === channel && msg.status === 'running'
    );
    expect(received).toMatchObject({ channel, ...payload });

    ws.close();
    await waitForClose(ws).catch(() => {});
  });

  it('keeps messages isolated by groupId', async () => {
    const groupA = randomUUID();
    const groupB = randomUUID();
    const userA = randomUUID();
    const userB = randomUUID();
    const channel = `job:${randomUUID()}`;

    const wsA = new WebSocketImpl(baseUrl, [token(userA, groupA)]);
    const wsB = new WebSocketImpl(baseUrl, [token(userB, groupB)]);
    await Promise.all([waitForReady(wsA), waitForReady(wsB)]);

    wsA.send(JSON.stringify({ type: 'subscribe', channel }));
    wsB.send(JSON.stringify({ type: 'subscribe', channel }));
    await waitForMessage(wsA, (msg) => msg.type === 'subscribed' && msg.channel === channel);
    await waitForMessage(wsB, (msg) => msg.type === 'subscribed' && msg.channel === channel);

    const receiptA = waitForMessage(wsA, (msg) => msg.channel === channel && msg.status === 'queued');
    const stayQuietB = new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => resolve(), 500);
      wsB.addEventListener('message', () => {
        clearTimeout(timer);
        reject(new Error('group B should not receive the message'));
      });
      wsB.addEventListener('close', () => {
        clearTimeout(timer);
        resolve();
      });
    });

    await publishWs(channel, { status: 'queued' }, { groupId: groupA });
    await receiptA;
    await stayQuietB;

    wsA.close();
    wsB.close();
    await Promise.all([waitForClose(wsA).catch(() => {}), waitForClose(wsB).catch(() => {})]);
  });
});
