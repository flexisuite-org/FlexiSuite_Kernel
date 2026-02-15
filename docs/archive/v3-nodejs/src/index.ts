import { buildServer } from './api/server';
import { config } from './config';
import { logger } from './lib/logger';
import { prisma } from './lib/db';
import { getRedis } from './lib/redis';

async function main() {
  // warm up connections
  await prisma.$queryRaw`SELECT 1`;
  await getRedis().ping();

  const app = buildServer();
  await app.listen({ port: config.port, host: '0.0.0.0' });
  logger.info(`FlexiSuite Kernel listening on ${config.port}`);
}

main().catch((err) => {
  logger.error({ err }, 'Failed to start server');
  process.exit(1);
});
