"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.default = healthRoutes;
const db_1 = require("../../lib/db");
const redis_1 = require("../../lib/redis");
async function healthRoutes(fastify) {
    fastify.get('/', async () => {
        const db = await db_1.prisma.$queryRaw `SELECT 1 as ok`;
        const redisPing = await (0, redis_1.getRedis)().ping();
        return {
            status: 'ok',
            db: Array.isArray(db) ? 'up' : 'unknown',
            redis: redisPing === 'PONG' ? 'up' : 'down'
        };
    });
}
//# sourceMappingURL=health.js.map