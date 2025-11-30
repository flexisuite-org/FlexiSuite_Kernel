"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.config = void 0;
const zod_1 = require("zod");
require("dotenv/config");
const schema = zod_1.z.object({
    NODE_ENV: zod_1.z.enum(['development', 'test', 'production']).default('development'),
    PORT: zod_1.z.string().default('9000'),
    DATABASE_URL: zod_1.z.string(),
    REDIS_URL: zod_1.z.string(),
    JWT_SECRET: zod_1.z.string().min(16),
    JWT_EXPIRES_IN: zod_1.z.string().default('15m'),
    REFRESH_TOKEN_SECRET: zod_1.z.string().min(16),
    REFRESH_TOKEN_EXPIRES_IN: zod_1.z.string().default('7d'),
    SIGNING_SECRET: zod_1.z.string().optional(),
    GITHUB_WEBHOOK_SECRET: zod_1.z.string().optional(),
    STORAGE_DRIVER: zod_1.z.enum(['local', 's3']).default('local'),
    BUNDLE_STORAGE_LOCAL_DIR: zod_1.z.string().default('storage/bundles'),
    S3_BUCKET: zod_1.z.string().optional(),
    S3_REGION: zod_1.z.string().default('us-east-1'),
    S3_ENDPOINT: zod_1.z.string().optional(),
    S3_FORCE_PATH_STYLE: zod_1.z.string().default('false'),
    S3_ACCESS_KEY_ID: zod_1.z.string().optional(),
    S3_SECRET_ACCESS_KEY: zod_1.z.string().optional(),
    RATE_LIMIT_MAX: zod_1.z.string().default('100'),
    RATE_LIMIT_WINDOW: zod_1.z.string().default('60000'),
    SANDBOX_MEMORY_MB: zod_1.z.string().default('128'),
    SANDBOX_TIMEOUT_MS: zod_1.z.string().default('500'),
    LOG_LEVEL: zod_1.z.string().default('info'),
    CAPABILITY_ALLOWLIST: zod_1.z.string().optional(),
    CAPABILITY_ROLE_ALLOWLIST: zod_1.z.string().optional(),
    // LLM providers
    OPENAI_API_KEY: zod_1.z.string().optional(),
    OPENAI_API_BASE: zod_1.z.string().optional(),
    OPENAI_DEFAULT_MODEL: zod_1.z.string().default('gpt-4o-mini'),
    GEMINI_API_KEY: zod_1.z.string().optional(),
    GEMINI_API_BASE: zod_1.z.string().default('https://generativelanguage.googleapis.com'),
    GEMINI_DEFAULT_MODEL: zod_1.z.string().default('gemini-1.5-flash'),
    // AI specific rate limit (per group/user)
    AI_RATE_LIMIT_MAX: zod_1.z.string().default('60'),
    AI_RATE_LIMIT_WINDOW: zod_1.z.string().default('300000')
});
const parsed = schema.safeParse(process.env);
if (!parsed.success) {
    console.error('Invalid environment configuration', parsed.error.flatten());
    process.exit(1);
}
const signingSecret = parsed.data.SIGNING_SECRET || (parsed.data.NODE_ENV === 'test' ? 'testsecret' : undefined);
const webhookSecret = parsed.data.GITHUB_WEBHOOK_SECRET || (parsed.data.NODE_ENV === 'test' ? 'testhooksecret' : undefined);
let capabilityRoleAllowlist = {};
if (parsed.data.CAPABILITY_ROLE_ALLOWLIST) {
    try {
        const parsedJson = JSON.parse(parsed.data.CAPABILITY_ROLE_ALLOWLIST);
        if (parsedJson && typeof parsedJson === 'object') {
            capabilityRoleAllowlist = Object.fromEntries(Object.entries(parsedJson).map(([k, v]) => [k, Array.isArray(v) ? v : []]));
        }
    }
    catch {
        console.warn('CAPABILITY_ROLE_ALLOWLIST is not valid JSON, falling back to empty map');
    }
}
exports.config = {
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
//# sourceMappingURL=config.js.map