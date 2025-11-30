"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.default = aiRoutes;
const zod_1 = require("zod");
const ai_1 = require("../../lib/ai");
const request_context_1 = require("../../lib/request-context");
const config_1 = require("../../config");
const messageSchema = zod_1.z.object({
    role: zod_1.z.enum(['system', 'user', 'assistant', 'tool']),
    content: zod_1.z.string().min(1)
});
const chatSchema = zod_1.z.object({
    provider: zod_1.z.enum(['openai', 'gemini']).optional(),
    model: zod_1.z.string().optional(),
    messages: zod_1.z.array(messageSchema).min(1),
    stream: zod_1.z.boolean().optional(),
    temperature: zod_1.z.number().min(0).max(2).optional(),
    max_tokens: zod_1.z.number().int().positive().optional(),
    apiKey: zod_1.z.string().min(1).max(500).optional()
});
function apiKeyFor(provider, override) {
    if (override)
        return override;
    return provider === 'openai' ? config_1.config.openai.apiKey : config_1.config.gemini.apiKey;
}
async function handleUsageLog(params) {
    try {
        await (0, ai_1.recordAiUsage)(params);
    }
    catch {
        // ignore audit failures to avoid blocking the main flow
    }
}
async function aiRoutes(fastify) {
    fastify.post('/chat', async (req, reply) => {
        const ctx = request_context_1.requestContext.getStore();
        if (!ctx?.groupId || !ctx?.userId) {
            return reply.code(401).send({ error: 'unauthorized', message: 'groupId and user required' });
        }
        const parsed = chatSchema.safeParse(req.body);
        if (!parsed.success) {
            return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
        }
        const body = parsed.data;
        const provider = (0, ai_1.inferProvider)(body.provider, body.model);
        const model = body.model || (0, ai_1.defaultModelFor)(provider);
        const apiKey = apiKeyFor(provider, body.apiKey);
        const stream = body.stream ?? false;
        if (!apiKey) {
            return reply.code(400).send({
                error: 'missing_api_key',
                message: `Provide apiKey in request or set ${provider.toUpperCase()}_API_KEY env`
            });
        }
        // Simple per-group and per-user token bucket (60 req/5min by default)
        const limitKeys = [`grp:${ctx.groupId}`, `grp:${ctx.groupId}:user:${ctx.userId}`];
        for (const key of limitKeys) {
            const hit = ai_1.aiRateLimiter.consume(key);
            if (!hit.allowed) {
                return reply
                    .code(429)
                    .header('Retry-After', Math.max(1, Math.ceil((hit.resetAt - Date.now()) / 1000)))
                    .send({ error: 'rate_limited', retryAt: new Date(hit.resetAt).toISOString() });
            }
        }
        // First pass: always return non-stream responses even if stream=true
        if (stream) {
            reply.header('x-stream-disabled', 'true');
        }
        try {
            const result = await (0, ai_1.callChatCompletion)({
                provider,
                model,
                messages: body.messages,
                temperature: body.temperature,
                maxTokens: body.max_tokens,
                apiKey,
                apiBase: provider === 'openai' ? config_1.config.openai.apiBase : config_1.config.gemini.apiBase,
                stream: false
            });
            await handleUsageLog({
                groupId: ctx.groupId,
                userId: ctx.userId,
                provider,
                model: result.model,
                usage: result.usage,
                success: true,
                usedOverrideKey: Boolean(body.apiKey)
            });
            return reply.send({ ...result, stream: false });
        }
        catch (err) {
            if (err instanceof ai_1.ProviderHttpError) {
                await handleUsageLog({
                    groupId: ctx.groupId,
                    userId: ctx.userId,
                    provider,
                    model,
                    success: false,
                    usedOverrideKey: Boolean(body.apiKey),
                    usage: undefined
                });
                return reply
                    .code(502)
                    .send({ error: 'upstream_error', provider: err.provider, status: err.status, message: err.message });
            }
            await handleUsageLog({
                groupId: ctx.groupId,
                userId: ctx.userId,
                provider,
                model,
                success: false,
                usedOverrideKey: Boolean(body.apiKey)
            });
            return reply.code(500).send({ error: 'internal_error' });
        }
    });
}
//# sourceMappingURL=ai.js.map