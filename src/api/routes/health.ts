import { FastifyInstance } from 'fastify';
import { prisma } from '../../lib/db';
import { redis } from '../../lib/redis';

export default async function healthRoutes(fastify: FastifyInstance) {
  fastify.get('/', async () => {
    const db = await prisma.$queryRaw`SELECT 1 as ok`;
    const redisPing = await redis.ping();
    return {
      status: 'ok',
      db: Array.isArray(db) ? 'up' : 'unknown',
      redis: redisPing === 'PONG' ? 'up' : 'down'
    };
  });
}
