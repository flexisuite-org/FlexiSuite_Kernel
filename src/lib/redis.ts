import { Redis } from 'ioredis';
import { config } from '../config';
import { logger } from './logger';

// Lazy singleton to avoid creating a client during module import (helps Jest exit cleanly)
let redisInstance: Redis | null = null;

export function getRedis() {
  if (!redisInstance) {
    redisInstance = new Redis(config.REDIS_URL, {
      maxRetriesPerRequest: null
    });
    redisInstance.on('error', (err) => logger.error({ err }, 'Redis error'));
    redisInstance.on('connect', () => logger.info('Redis connected'));
  }
  return redisInstance;
}

export async function closeRedis() {
  if (redisInstance && redisInstance.status !== 'end') {
    await redisInstance.quit();
  }
  redisInstance = null;
}
