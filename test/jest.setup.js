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
Object.defineProperty(exports, "__esModule", { value: true });
process.env.NODE_ENV = process.env.NODE_ENV || 'test';
const request_context_1 = require("../src/lib/request-context");
const db_1 = require("../src/lib/db");
const seed_1 = require("./helpers/seed");
process.env.SIGNING_SECRET = process.env.SIGNING_SECRET || 'testsecret';
process.env.GITHUB_WEBHOOK_SECRET = process.env.GITHUB_WEBHOOK_SECRET || 'testhooksecret';
jest.setTimeout(20000);
beforeEach(async () => {
    request_context_1.requestContext.disable?.();
    await (0, seed_1.truncateAll)();
});
afterAll(async () => {
    try {
        const { closeRedis } = await Promise.resolve().then(() => __importStar(require('../src/lib/redis')));
        await closeRedis();
    }
    catch {
        /* ignore */
    }
    await db_1.prisma.$disconnect().catch(() => { });
});
//# sourceMappingURL=jest.setup.js.map