import { z } from 'zod';
import 'dotenv/config';

const schema = z.object({
  NODE_ENV: z.enum(['development', 'test', 'production']).default('development'),
  PORT: z.string().default('3000'),
  DATABASE_URL: z.string(),
  REDIS_URL: z.string(),
  JWT_SECRET: z.string().min(16),
  JWT_EXPIRES_IN: z.string().default('15m'),
  REFRESH_TOKEN_SECRET: z.string().min(16),
  REFRESH_TOKEN_EXPIRES_IN: z.string().default('7d'),
  RATE_LIMIT_MAX: z.string().default('100'),
  RATE_LIMIT_WINDOW: z.string().default('60000'),
  SANDBOX_MEMORY_MB: z.string().default('128'),
  SANDBOX_TIMEOUT_MS: z.string().default('500'),
  LOG_LEVEL: z.string().default('info')
});

const parsed = schema.safeParse(process.env);

if (!parsed.success) {
  console.error('Invalid environment configuration', parsed.error.flatten());
  process.exit(1);
}

export const config = {
  ...parsed.data,
  port: parseInt(parsed.data.PORT, 10),
  rateLimit: {
    max: parseInt(parsed.data.RATE_LIMIT_MAX, 10),
    windowMs: parseInt(parsed.data.RATE_LIMIT_WINDOW, 10)
  },
  sandbox: {
    memoryMb: parseInt(parsed.data.SANDBOX_MEMORY_MB, 10),
    timeoutMs: parseInt(parsed.data.SANDBOX_TIMEOUT_MS, 10)
  }
};
