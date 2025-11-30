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
const crypto_1 = __importDefault(require("crypto"));
const server_1 = require("../src/api/server");
const db_1 = require("../src/lib/db");
const seed_1 = require("./helpers/seed");
describe('github webhook signature', () => {
    const app = (0, server_1.buildServer)();
    beforeEach(async () => {
        await app.ready();
        await (0, seed_1.truncateAll)();
    });
    afterAll(async () => {
        await db_1.prisma.$disconnect();
        await app.close();
        const { closeRedis } = await Promise.resolve().then(() => __importStar(require('../src/lib/redis')));
        await closeRedis();
    });
    function signedBody(payload, secret) {
        const raw = JSON.stringify(payload);
        const sig = 'sha256=' + crypto_1.default.createHmac('sha256', secret).update(raw).digest('hex');
        return { raw, sig };
    }
    it('rejects missing or invalid signature when secret is set', async () => {
        const secret = process.env.GITHUB_WEBHOOK_SECRET || 'testhooksecret';
        const payload = { ref: 'refs/heads/main', repository: { full_name: 'demo/repo' } };
        const { raw } = signedBody(payload, secret);
        const res = await (0, supertest_1.default)(app.server)
            .post('/integrations/github/webhook')
            .set('content-type', 'application/json')
            .send(raw);
        expect(res.status).toBe(401);
    });
    it('accepts valid signature', async () => {
        const secret = process.env.GITHUB_WEBHOOK_SECRET || 'testhooksecret';
        const payload = { ref: 'refs/heads/main', repository: { full_name: 'demo/repo' }, head_commit: { id: 'abc' } };
        const { raw, sig } = signedBody(payload, secret);
        const res = await (0, supertest_1.default)(app.server)
            .post('/integrations/github/webhook')
            .set('content-type', 'application/json')
            .set('x-hub-signature-256', sig)
            .send(raw);
        expect(res.status).toBe(202);
    });
});
//# sourceMappingURL=github.webhook.spec.js.map