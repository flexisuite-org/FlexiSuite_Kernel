"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.getRedis = getRedis;
exports.closeRedis = closeRedis;
const ioredis_1 = require("ioredis");
const config_1 = require("../config");
const logger_1 = require("./logger");
// Lazy singleton to avoid creating a client during module import (helps Jest exit cleanly)
let redisInstance = null;
function getRedis() {
    if (!redisInstance) {
        redisInstance = new ioredis_1.Redis(config_1.config.REDIS_URL, {
            maxRetriesPerRequest: null
        });
        redisInstance.on('error', (err) => logger_1.logger.error({ err }, 'Redis error'));
        redisInstance.on('connect', () => logger_1.logger.info('Redis connected'));
    }
    return redisInstance;
}
async function closeRedis() {
    if (redisInstance && redisInstance.status !== 'end') {
        await redisInstance.quit();
    }
    redisInstance = null;
}
//# sourceMappingURL=redis.js.map