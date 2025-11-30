"use strict";
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const crypto_1 = require("crypto");
const server_1 = require("../src/api/server");
const config_1 = require("../src/config");
const ws_bus_1 = require("../src/lib/ws-bus");
const db_1 = require("../src/lib/db");
const redis_1 = require("../src/lib/redis");
const WebSocketImpl = globalThis.WebSocket;
function token(userId, groupId, roles = []) {
    return jsonwebtoken_1.default.sign({ userId, groupId, roles }, config_1.config.JWT_SECRET);
}
function waitForClose(ws, timeout = 1500) {
    return new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('close timeout')), timeout);
        ws.addEventListener('close', (evt) => {
            clearTimeout(timer);
            resolve(evt);
        });
        ws.addEventListener('error', (err) => {
            clearTimeout(timer);
            reject(err);
        });
    });
}
function waitForMessage(ws, matcher, timeout = 2000) {
    return new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('message timeout')), timeout);
        ws.addEventListener('message', (evt) => {
            try {
                const raw = typeof evt.data === 'string' ? evt.data : Buffer.from(evt.data).toString();
                const parsed = JSON.parse(raw);
                if (matcher(parsed)) {
                    clearTimeout(timer);
                    resolve(parsed);
                }
            }
            catch (err) {
                clearTimeout(timer);
                reject(err);
            }
        });
        ws.addEventListener('close', (evt) => {
            clearTimeout(timer);
            reject(new Error(`socket closed early: ${evt.code}`));
        });
    });
}
async function waitForReady(ws) {
    return waitForMessage(ws, (msg) => msg.type === 'ready');
}
describe('websocket channels', () => {
    const app = (0, server_1.buildServer)();
    let baseUrl;
    beforeAll(async () => {
        await app.listen({ port: 0 });
        const address = app.server.address();
        baseUrl = `ws://127.0.0.1:${address.port}/ws`;
    });
    afterAll(async () => {
        await app.close();
        await db_1.prisma.$disconnect().catch(() => { });
        await (0, redis_1.closeRedis)().catch(() => { });
    });
    it('rejects connections without JWT', async () => {
        const ws = new WebSocketImpl(baseUrl);
        const closeEvt = await waitForClose(ws);
        expect(closeEvt.code).toBe(1008);
    });
    it('delivers published message to subscribed channel', async () => {
        const groupId = (0, crypto_1.randomUUID)();
        const userId = (0, crypto_1.randomUUID)();
        const channel = `job:${(0, crypto_1.randomUUID)()}`;
        const ws = new WebSocketImpl(baseUrl, [token(userId, groupId)]);
        await waitForReady(ws);
        ws.send(JSON.stringify({ type: 'subscribe', channel }));
        await waitForMessage(ws, (msg) => msg.type === 'subscribed' && msg.channel === channel);
        const payload = { status: 'running', message: 'starting', step: 1 };
        await (0, ws_bus_1.publishWs)(channel, payload, { groupId });
        const received = await waitForMessage(ws, (msg) => msg.channel === channel && msg.status === 'running');
        expect(received).toMatchObject({ channel, ...payload });
        ws.close();
        await waitForClose(ws).catch(() => { });
    });
    it('keeps messages isolated by groupId', async () => {
        const groupA = (0, crypto_1.randomUUID)();
        const groupB = (0, crypto_1.randomUUID)();
        const userA = (0, crypto_1.randomUUID)();
        const userB = (0, crypto_1.randomUUID)();
        const channel = `job:${(0, crypto_1.randomUUID)()}`;
        const wsA = new WebSocketImpl(baseUrl, [token(userA, groupA)]);
        const wsB = new WebSocketImpl(baseUrl, [token(userB, groupB)]);
        await Promise.all([waitForReady(wsA), waitForReady(wsB)]);
        wsA.send(JSON.stringify({ type: 'subscribe', channel }));
        wsB.send(JSON.stringify({ type: 'subscribe', channel }));
        await waitForMessage(wsA, (msg) => msg.type === 'subscribed' && msg.channel === channel);
        await waitForMessage(wsB, (msg) => msg.type === 'subscribed' && msg.channel === channel);
        const receiptA = waitForMessage(wsA, (msg) => msg.channel === channel && msg.status === 'queued');
        const stayQuietB = new Promise((resolve, reject) => {
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
        await (0, ws_bus_1.publishWs)(channel, { status: 'queued' }, { groupId: groupA });
        await receiptA;
        await stayQuietB;
        wsA.close();
        wsB.close();
        await Promise.all([waitForClose(wsA).catch(() => { }), waitForClose(wsB).catch(() => { })]);
    });
});
//# sourceMappingURL=websocket.channels.spec.js.map