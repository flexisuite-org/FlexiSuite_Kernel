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
const server_1 = require("../src/api/server");
const supertest_1 = __importDefault(require("supertest"));
const jsonwebtoken_1 = __importDefault(require("jsonwebtoken"));
const db_1 = require("../src/lib/db");
const config_1 = require("../src/config");
const seed_1 = require("./helpers/seed");
function token(userId, groupId) {
    return jsonwebtoken_1.default.sign({ userId, groupId, roles: [] }, config_1.config.JWT_SECRET);
}
describe('draft sandbox write guard', () => {
    const app = (0, server_1.buildServer)();
    let groupId;
    let userId;
    beforeEach(async () => {
        await app.ready();
        const seed = await (0, seed_1.createTenantSeed)(`draft-${Date.now()}`);
        groupId = seed.groupId;
        userId = seed.userId;
    });
    afterAll(async () => {
        await db_1.prisma.$disconnect();
        await app.close();
        const { closeRedis } = await Promise.resolve().then(() => __importStar(require('../src/lib/redis')));
        await closeRedis();
    });
    it('blocks prisma writes inside draft sandbox', async () => {
        const script = `await kernel.prisma?.entityRecord?.create({data:{}});`;
        const res = await (0, supertest_1.default)(app.server)
            .post('/sandbox/drafts/run')
            .set('authorization', 'Bearer ' + token(userId, groupId))
            .send({ script });
        expect([401, 500]).toContain(res.status);
    });
});
//# sourceMappingURL=drafts.writeguard.spec.js.map