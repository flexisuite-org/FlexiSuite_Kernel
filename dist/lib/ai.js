"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.aiRateLimiter = exports.SlidingWindowLimiter = exports.ProviderHttpError = void 0;
exports.inferProvider = inferProvider;
exports.defaultModelFor = defaultModelFor;
exports.callChatCompletion = callChatCompletion;
exports.recordAiUsage = recordAiUsage;
const db_1 = require("./db");
const config_1 = require("../config");
class ProviderHttpError extends Error {
    constructor(provider, status, body, message) {
        super(message || `provider ${provider} responded with status ${status}`);
        this.provider = provider;
        this.status = status;
        this.body = body;
    }
}
exports.ProviderHttpError = ProviderHttpError;
// Simple in-memory sliding window counter for per-tenant rate limiting.
class SlidingWindowLimiter {
    constructor(limit, windowMs) {
        this.buckets = new Map();
        this.limit = limit;
        this.windowMs = windowMs;
    }
    consume(key, now = Date.now()) {
        let bucket = this.buckets.get(key);
        if (!bucket || bucket.resetAt <= now) {
            bucket = { count: 0, resetAt: now + this.windowMs };
        }
        if (bucket.count >= this.limit) {
            this.buckets.set(key, bucket);
            return { allowed: false, remaining: 0, resetAt: bucket.resetAt };
        }
        bucket.count += 1;
        this.buckets.set(key, bucket);
        return { allowed: true, remaining: this.limit - bucket.count, resetAt: bucket.resetAt };
    }
    seed(key, count, resetAt) {
        const now = Date.now();
        const bucket = { count, resetAt: resetAt ?? now + this.windowMs };
        this.buckets.set(key, bucket);
    }
    reset(key) {
        if (key)
            this.buckets.delete(key);
        else
            this.buckets.clear();
    }
    setLimit(limit, windowMs) {
        this.limit = limit;
        if (windowMs)
            this.windowMs = windowMs;
    }
}
exports.SlidingWindowLimiter = SlidingWindowLimiter;
exports.aiRateLimiter = new SlidingWindowLimiter(config_1.config.aiRateLimit.max, config_1.config.aiRateLimit.windowMs);
function inferProvider(provider, model) {
    if (provider)
        return provider;
    if (model && /gemini/i.test(model))
        return 'gemini';
    return 'openai';
}
function defaultModelFor(provider) {
    return provider === 'openai' ? config_1.config.openai.defaultModel : config_1.config.gemini.defaultModel;
}
function openAiUrl(apiBase) {
    return `${apiBase || 'https://api.openai.com/v1'}/chat/completions`;
}
function geminiUrl(model, apiBase) {
    const base = apiBase || 'https://generativelanguage.googleapis.com';
    const normalized = model.startsWith('models/') ? model : `models/${model}`;
    // Non-streaming endpoint for first pass
    return `${base}/v1beta/${normalized}:generateContent`;
}
function parseOpenAiError(bodyText) {
    try {
        const json = JSON.parse(bodyText);
        return json.error?.message || bodyText;
    }
    catch {
        return bodyText;
    }
}
function parseGeminiError(bodyText) {
    try {
        const json = JSON.parse(bodyText);
        return json.error?.message || bodyText;
    }
    catch {
        return bodyText;
    }
}
function toGeminiPayload(messages, temperature, maxTokens) {
    const systemParts = [];
    const contents = [];
    messages.forEach((m) => {
        if (m.role === 'system') {
            systemParts.push(m.content);
            return;
        }
        const role = m.role === 'assistant' ? 'model' : 'user';
        contents.push({ role, parts: [{ text: m.content }] });
    });
    const payload = { contents };
    if (systemParts.length) {
        payload.systemInstruction = { role: 'system', parts: [{ text: systemParts.join('\n') }] };
    }
    payload.generationConfig = {};
    if (typeof temperature === 'number')
        payload.generationConfig.temperature = temperature;
    if (typeof maxTokens === 'number')
        payload.generationConfig.maxOutputTokens = maxTokens;
    if (Object.keys(payload.generationConfig).length === 0)
        delete payload.generationConfig;
    return payload;
}
function normalizeGeminiText(candidate) {
    if (!candidate?.content?.parts)
        return '';
    return candidate.content.parts.map((p) => p.text || '').join('');
}
async function callChatCompletion(opts) {
    if (opts.provider === 'openai') {
        const res = await fetch(openAiUrl(opts.apiBase), {
            method: 'POST',
            headers: {
                'content-type': 'application/json',
                authorization: `Bearer ${opts.apiKey}`
            },
            body: JSON.stringify({
                model: opts.model,
                messages: opts.messages,
                temperature: opts.temperature,
                max_tokens: opts.maxTokens,
                stream: opts.stream ?? false
            })
        });
        if (!res.ok) {
            const bodyText = await res.text();
            throw new ProviderHttpError('openai', res.status, bodyText, parseOpenAiError(bodyText));
        }
        const json = await res.json();
        return {
            provider: 'openai',
            model: json.model || opts.model,
            choices: json.choices,
            usage: json.usage,
            raw: json
        };
    }
    // Gemini (Generative Language API)
    const url = `${geminiUrl(opts.model, opts.apiBase)}?key=${encodeURIComponent(opts.apiKey)}`;
    const res = await fetch(url, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(toGeminiPayload(opts.messages, opts.temperature, opts.maxTokens))
    });
    if (!res.ok) {
        const bodyText = await res.text();
        throw new ProviderHttpError('gemini', res.status, bodyText, parseGeminiError(bodyText));
    }
    const json = await res.json();
    const first = json.candidates?.[0];
    const text = normalizeGeminiText(first);
    const usage = json.usageMetadata || {};
    return {
        provider: 'gemini',
        model: json.model || opts.model,
        choices: [
            {
                index: 0,
                message: { role: 'assistant', content: text },
                finish_reason: first?.finishReason || 'stop'
            }
        ],
        usage: {
            prompt_tokens: usage.promptTokenCount,
            completion_tokens: usage.candidatesTokenCount,
            total_tokens: usage.totalTokenCount
        },
        raw: json
    };
}
async function recordAiUsage(params) {
    const metadata = {
        provider: params.provider,
        model: params.model,
        prompt_tokens: params.usage?.prompt_tokens,
        completion_tokens: params.usage?.completion_tokens,
        total_tokens: params.usage?.total_tokens,
        usedOverrideKey: params.usedOverrideKey
    };
    await db_1.prisma.auditLog.create({
        data: {
            actorUserId: params.userId ?? undefined,
            groupId: params.groupId,
            resource: 'ai.chat',
            action: `proxy.${params.provider}`,
            metadata,
            success: params.success
        }
    });
}
//# sourceMappingURL=ai.js.map