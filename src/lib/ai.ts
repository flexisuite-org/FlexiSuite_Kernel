import { prisma } from './db';
import { config } from '../config';

export type ProviderName = 'openai' | 'gemini';

export interface ChatMessage {
  role: 'system' | 'user' | 'assistant' | 'tool';
  content: string;
}

export interface ChatCallOptions {
  provider: ProviderName;
  model: string;
  messages: ChatMessage[];
  temperature?: number;
  maxTokens?: number;
  apiKey: string;
  apiBase?: string;
  stream?: boolean;
}

export interface ChatCompletionResult {
  provider: ProviderName;
  model: string;
  choices: any[];
  usage?: { prompt_tokens?: number; completion_tokens?: number; total_tokens?: number };
  raw: any;
}

export class ProviderHttpError extends Error {
  provider: ProviderName;
  status: number;
  body: any;

  constructor(provider: ProviderName, status: number, body: any, message?: string) {
    super(message || `provider ${provider} responded with status ${status}`);
    this.provider = provider;
    this.status = status;
    this.body = body;
  }
}

type Bucket = { count: number; resetAt: number };

// Simple in-memory sliding window counter for per-tenant rate limiting.
export class SlidingWindowLimiter {
  private limit: number;
  private windowMs: number;
  private buckets = new Map<string, Bucket>();

  constructor(limit: number, windowMs: number) {
    this.limit = limit;
    this.windowMs = windowMs;
  }

  consume(key: string, now = Date.now()) {
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

  seed(key: string, count: number, resetAt?: number) {
    const now = Date.now();
    const bucket: Bucket = { count, resetAt: resetAt ?? now + this.windowMs };
    this.buckets.set(key, bucket);
  }

  reset(key?: string) {
    if (key) this.buckets.delete(key);
    else this.buckets.clear();
  }

  setLimit(limit: number, windowMs?: number) {
    this.limit = limit;
    if (windowMs) this.windowMs = windowMs;
  }
}

export const aiRateLimiter = new SlidingWindowLimiter(config.aiRateLimit.max, config.aiRateLimit.windowMs);

export function inferProvider(provider?: ProviderName, model?: string): ProviderName {
  if (provider) return provider;
  if (model && /gemini/i.test(model)) return 'gemini';
  return 'openai';
}

export function defaultModelFor(provider: ProviderName) {
  return provider === 'openai' ? config.openai.defaultModel : config.gemini.defaultModel;
}

function openAiUrl(apiBase?: string) {
  return `${apiBase || 'https://api.openai.com/v1'}/chat/completions`;
}

function geminiUrl(model: string, apiBase?: string) {
  const base = apiBase || 'https://generativelanguage.googleapis.com';
  const normalized = model.startsWith('models/') ? model : `models/${model}`;
  // Non-streaming endpoint for first pass
  return `${base}/v1beta/${normalized}:generateContent`;
}

function parseOpenAiError(bodyText: string) {
  try {
    const json = JSON.parse(bodyText);
    return json.error?.message || bodyText;
  } catch {
    return bodyText;
  }
}

function parseGeminiError(bodyText: string) {
  try {
    const json = JSON.parse(bodyText);
    return json.error?.message || bodyText;
  } catch {
    return bodyText;
  }
}

function toGeminiPayload(messages: ChatMessage[], temperature?: number, maxTokens?: number) {
  const systemParts: string[] = [];
  const contents: any[] = [];

  messages.forEach((m) => {
    if (m.role === 'system') {
      systemParts.push(m.content);
      return;
    }
    const role = m.role === 'assistant' ? 'model' : 'user';
    contents.push({ role, parts: [{ text: m.content }] });
  });

  const payload: any = { contents };
  if (systemParts.length) {
    payload.systemInstruction = { role: 'system', parts: [{ text: systemParts.join('\n') }] };
  }

  payload.generationConfig = {} as any;
  if (typeof temperature === 'number') payload.generationConfig.temperature = temperature;
  if (typeof maxTokens === 'number') payload.generationConfig.maxOutputTokens = maxTokens;
  if (Object.keys(payload.generationConfig).length === 0) delete payload.generationConfig;

  return payload;
}

function normalizeGeminiText(candidate: any) {
  if (!candidate?.content?.parts) return '';
  return candidate.content.parts.map((p: any) => p.text || '').join('');
}

export async function callChatCompletion(opts: ChatCallOptions): Promise<ChatCompletionResult> {
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

export async function recordAiUsage(params: {
  groupId: string;
  userId?: string | null;
  provider: ProviderName;
  model: string;
  usage?: { prompt_tokens?: number; completion_tokens?: number; total_tokens?: number };
  success: boolean;
  usedOverrideKey: boolean;
}) {
  const metadata: any = {
    provider: params.provider,
    model: params.model,
    prompt_tokens: params.usage?.prompt_tokens,
    completion_tokens: params.usage?.completion_tokens,
    total_tokens: params.usage?.total_tokens,
    usedOverrideKey: params.usedOverrideKey
  };

  await prisma.auditLog.create({
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
