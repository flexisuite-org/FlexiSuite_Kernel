"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
const server_1 = require("./api/server");
const config_1 = require("./config");
const logger_1 = require("./lib/logger");
const db_1 = require("./lib/db");
const redis_1 = require("./lib/redis");
async function main() {
    // warm up connections
    await db_1.prisma.$queryRaw `SELECT 1`;
    await (0, redis_1.getRedis)().ping();
    const app = (0, server_1.buildServer)();
    await app.listen({ port: config_1.config.port, host: '0.0.0.0' });
    logger_1.logger.info(`FlexiSuite Kernel listening on ${config_1.config.port}`);
}
main().catch((err) => {
    logger_1.logger.error({ err }, 'Failed to start server');
    process.exit(1);
});
//# sourceMappingURL=index.js.map