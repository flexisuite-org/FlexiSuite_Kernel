import pino from 'pino';
import { config } from '../config';

export const logger = pino({
  level: config.LOG_LEVEL,
  redact: ['password', 'passwordHash', 'token', 'authorization', 'refreshToken'],
  transport: process.env.NODE_ENV === 'development' ? { target: 'pino-pretty' } : undefined
});
