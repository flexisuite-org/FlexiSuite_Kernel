"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const supertest_1 = __importDefault(require("supertest"));
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const server_1 = require("../src/api/server");
const ai_1 = require("../src/lib/ai");
const config_1 = require("../src/config");
const seed_1 = require("./helpers/seed");
const db_1 = require("../src/lib/db");
const app = (0, server_1.buildServer)();
function token(userId, groupId) {
    return jsonwebtoken_1.default.sign({ userId, groupId, roles: [] }, config_1.config.JWT_SECRET);
}
describe('AI proxy /ai/chat', () => {
    beforeAll(async () => {
        await app.ready();
    });
    beforeEach(() => {
        ai_1.aiRateLimiter.reset();
    });
    afterAll(async () => {
        await db_1.prisma.$disconnect();
        await app.close();
        const { closeRedis } = await Promise.resolve().then(() => __importStar(require('../src/lib/redis')));
        await closeRedis();
    });
    it('rejects when groupId is missing', async () => {
        const res = await (0, supertest_1.default)(app.server).post('/ai/chat').send({ messages: [{ role: 'user', content: 'hi' }] });
        expect(res.status).toBe(401);
    });
    it('proxies to OpenAI using provided apiKey and logs usage', async () => {
        const { groupId, userId } = await (0, seed_1.createTenantSeed)(`ai-${Date.now()}`);
        const fetchMock = jest.spyOn(global, 'fetch').mockResolvedValue(new global.Response(JSON.stringify({
            id: 'chatcmpl-test',
            choices: [
                { index: 0, message: { role: 'assistant', content: 'hello!' }, finish_reason: 'stop' }
            ],
            usage: { prompt_tokens: 3, completion_tokens: 5, total_tokens: 8 },
            model: 'gpt-4o-mini'
        }), { status: 200, headers: { 'Content-Type': 'application/json' } }));
        const res = await (0, supertest_1.default)(app.server)
            .post('/ai/chat')
            .set('authorization', 'Bearer ' + token(userId, groupId))
            .send({ messages: [{ role: 'user', content: 'hello' }], apiKey: 'user-openai-key' });
        expect(res.status).toBe(200);
        expect(res.body.choices[0].message.content).toBe('hello!');
        expect(fetchMock).toHaveBeenCalled();
        const logs = await db_1.prisma.auditLog.findMany({ where: { groupId, resource: 'ai.chat' } });
        expect(logs.length).toBe(1);
        expect(logs[0].metadata?.provider).toBe('openai');
        fetchMock.mockRestore();
    });
    it('infers Gemini from model name and uses override key', async () => {
        const { groupId, userId } = await (0, seed_1.createTenantSeed)(`gem-${Date.now()}`);
        const fetchMock = jest.spyOn(global, 'fetch').mockImplementation((input) => {
            expect(String(input)).toContain('gemini');
            return Promise.resolve(new global.Response(JSON.stringify({
                candidates: [{ content: { parts: [{ text: 'gemini response' }] } }],
                usageMetadata: { promptTokenCount: 2, candidatesTokenCount: 3, totalTokenCount: 5 },
                model: 'models/gemini-1.5-flash'
            }), { status: 200, headers: { 'Content-Type': 'application/json' } }));
        });
        const res = await (0, supertest_1.default)(app.server)
            .post('/ai/chat')
            .set('authorization', 'Bearer ' + token(userId, groupId))
            .send({ model: 'gemini-1.5-flash', messages: [{ role: 'user', content: 'hola' }], apiKey: 'gem-key' });
        expect(res.status).toBe(200);
        expect(res.body.choices[0].message.content).toBe('gemini response');
        expect(res.body.provider).toBe('gemini');
        fetchMock.mockRestore();
    });
    it('enforces per-group and per-user rate limits', async () => {
        const { groupId, userId } = await (0, seed_1.createTenantSeed)(`limit-${Date.now()}`);
        ai_1.aiRateLimiter.seed(`grp:${groupId}`, config_1.config.aiRateLimit.max);
        ai_1.aiRateLimiter.seed(`grp:${groupId}:user:${userId}`, config_1.config.aiRateLimit.max);
        const res = await (0, supertest_1.default)(app.server)
            .post('/ai/chat')
            .set('authorization', 'Bearer ' + token(userId, groupId))
            .send({ messages: [{ role: 'user', content: 'limit' }], apiKey: 'user-openai-key' });
        expect(res.status).toBe(429);
    });
});
//# sourceMappingURL=ai.proxy.spec.js.map