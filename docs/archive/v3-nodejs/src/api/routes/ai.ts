import { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import { z } from 'zod';
import {
  aiRateLimiter,
  callChatCompletion,
  defaultModelFor,
  inferProvider,
  ProviderHttpError,
  ProviderName,
  recordAiUsage
} from '../../lib/ai';
import { requestContext } from '../../lib/request-context';
import { config } from '../../config';

const messageSchema = z.object({
  role: z.enum(['system', 'user', 'assistant', 'tool']),
  content: z.string().min(1)
});

const chatSchema = z.object({
  provider: z.enum(['openai', 'gemini']).optional(),
  model: z.string().optional(),
  messages: z.array(messageSchema).min(1),
  stream: z.boolean().optional(),
  temperature: z.number().min(0).max(2).optional(),
  max_tokens: z.number().int().positive().optional(),
  apiKey: z.string().min(1).max(500).optional()
});

function apiKeyFor(provider: ProviderName, override?: string) {
  if (override) return override;
  return provider === 'openai' ? config.openai.apiKey : config.gemini.apiKey;
}

async function handleUsageLog(params: {
  groupId: string;
  userId: string;
  provider: ProviderName;
  model: string;
  success: boolean;
  usedOverrideKey: boolean;
  usage?: { prompt_tokens?: number; completion_tokens?: number; total_tokens?: number };
}) {
  try {
    await recordAiUsage(params);
  } catch {
    // ignore audit failures to avoid blocking the main flow
  }
}

export default async function aiRoutes(fastify: FastifyInstance) {
  fastify.post('/chat', async (req: FastifyRequest, reply: FastifyReply) => {
    const ctx = requestContext.getStore();
    if (!ctx?.groupId || !ctx?.userId) {
      return reply.code(401).send({ error: 'unauthorized', message: 'groupId and user required' });
    }

    const parsed = chatSchema.safeParse(req.body);
    if (!parsed.success) {
      return reply.code(400).send({ error: 'invalid_input', details: parsed.error.flatten() });
    }

    const body = parsed.data;
    const provider = inferProvider(body.provider as ProviderName | undefined, body.model);
    const model = body.model || defaultModelFor(provider);
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
      const hit = aiRateLimiter.consume(key);
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
      const result = await callChatCompletion({
        provider,
        model,
        messages: body.messages,
        temperature: body.temperature,
        maxTokens: body.max_tokens,
        apiKey,
        apiBase: provider === 'openai' ? config.openai.apiBase : config.gemini.apiBase,
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
    } catch (err: any) {
      if (err instanceof ProviderHttpError) {
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
