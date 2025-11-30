import { z } from 'zod';
import 'dotenv/config';

const schema = z.object({
  NODE_ENV: z.enum(['development', 'test', 'production']).default('development'),
  PORT: z.string().default('9000'),
  DATABASE_URL: z.string(),
  REDIS_URL: z.string(),
  JWT_SECRET: z.string().min(16),
  JWT_EXPIRES_IN: z.string().default('15m'),
  REFRESH_TOKEN_SECRET: z.string().min(16),
  REFRESH_TOKEN_EXPIRES_IN: z.string().default('7d'),
  SIGNING_SECRET: z.string().optional(),
  GITHUB_WEBHOOK_SECRET: z.string().optional(),
  STORAGE_DRIVER: z.enum(['local', 's3']).default('local'),
  BUNDLE_STORAGE_LOCAL_DIR: z.string().default('storage/bundles'),
  S3_BUCKET: z.string().optional(),
  S3_REGION: z.string().default('us-east-1'),
  S3_ENDPOINT: z.string().optional(),
  S3_FORCE_PATH_STYLE: z.string().default('false'),
  S3_ACCESS_KEY_ID: z.string().optional(),
  S3_SECRET_ACCESS_KEY: z.string().optional(),
  RATE_LIMIT_MAX: z.string().default('100'),
  RATE_LIMIT_WINDOW: z.string().default('60000'),
  SANDBOX_MEMORY_MB: z.string().default('128'),
  SANDBOX_TIMEOUT_MS: z.string().default('500'),
  LOG_LEVEL: z.string().default('info'),
  CAPABILITY_ALLOWLIST: z.string().optional(),
  CAPABILITY_ROLE_ALLOWLIST: z.string().optional(),

  // LLM providers
  OPENAI_API_KEY: z.string().optional(),
  OPENAI_API_BASE: z.string().optional(),
  OPENAI_DEFAULT_MODEL: z.string().default('gpt-4o-mini'),
  GEMINI_API_KEY: z.string().optional(),
  GEMINI_API_BASE: z.string().default('https://generativelanguage.googleapis.com'),
  GEMINI_DEFAULT_MODEL: z.string().default('gemini-1.5-flash'),

  // AI specific rate limit (per group/user)
  AI_RATE_LIMIT_MAX: z.string().default('60'),
  AI_RATE_LIMIT_WINDOW: z.string().default('300000')
});

const parsed = schema.safeParse(process.env);

if (!parsed.success) {
  console.error('Invalid environment configuration', parsed.error.flatten());
  process.exit(1);
}

const signingSecret =
  parsed.data.SIGNING_SECRET || (parsed.data.NODE_ENV === 'test' ? 'testsecret' : undefined);
const webhookSecret =
  parsed.data.GITHUB_WEBHOOK_SECRET || (parsed.data.NODE_ENV === 'test' ? 'testhooksecret' : undefined);

let capabilityRoleAllowlist: Record<string, string[]> = {};
if (parsed.data.CAPABILITY_ROLE_ALLOWLIST) {
  try {
    const parsedJson = JSON.parse(parsed.data.CAPABILITY_ROLE_ALLOWLIST);
    if (parsedJson && typeof parsedJson === 'object') {
      capabilityRoleAllowlist = Object.fromEntries(
        Object.entries(parsedJson).map(([k, v]) => [k, Array.isArray(v) ? v : []])
      );
    }
  } catch {
    console.warn('CAPABILITY_ROLE_ALLOWLIST is not valid JSON, falling back to empty map');
  }
}

export const config = {
  ...parsed.data,
  SIGNING_SECRET: signingSecret,
  GITHUB_WEBHOOK_SECRET: webhookSecret,
  port: parseInt(parsed.data.PORT, 10),
  rateLimit: {
    max: parseInt(parsed.data.RATE_LIMIT_MAX, 10),
    windowMs: parseInt(parsed.data.RATE_LIMIT_WINDOW, 10)
  },
  aiRateLimit: {
    max: parseInt(parsed.data.AI_RATE_LIMIT_MAX, 10),
    windowMs: parseInt(parsed.data.AI_RATE_LIMIT_WINDOW, 10)
  },
  openai: {
    apiKey: parsed.data.OPENAI_API_KEY,
    apiBase: parsed.data.OPENAI_API_BASE,
    defaultModel: parsed.data.OPENAI_DEFAULT_MODEL
  },
  gemini: {
    apiKey: parsed.data.GEMINI_API_KEY,
    apiBase: parsed.data.GEMINI_API_BASE,
    defaultModel: parsed.data.GEMINI_DEFAULT_MODEL
  },
  sandbox: {
    memoryMb: parseInt(parsed.data.SANDBOX_MEMORY_MB, 10),
    timeoutMs: parseInt(parsed.data.SANDBOX_TIMEOUT_MS, 10)
  },
  bundleStorage: {
    driver: parsed.data.STORAGE_DRIVER,
    localDir: parsed.data.BUNDLE_STORAGE_LOCAL_DIR,
    s3: {
      bucket: parsed.data.S3_BUCKET,
      region: parsed.data.S3_REGION,
      endpoint: parsed.data.S3_ENDPOINT,
      forcePathStyle: parsed.data.S3_FORCE_PATH_STYLE === 'true',
      accessKeyId: parsed.data.S3_ACCESS_KEY_ID,
      secretAccessKey: parsed.data.S3_SECRET_ACCESS_KEY
    }
  },
  capabilityAllowlist: (parsed.data.CAPABILITY_ALLOWLIST || 'echo,time.now,data.entity.get,data.entity.list,data.entity.listByDefinition,data.entity.getDefinition').split(','),
  capabilityRoleAllowlist
};
