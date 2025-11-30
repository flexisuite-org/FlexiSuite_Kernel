# Prompt: Implement OpenAI Proxy API for FlexiSuite Kernel

Goal
- Add a secure server-side proxy to call OpenAI Chat Completions so clients never see the API key and usage can be audited per group.

Context
- Stack: Fastify 5, Node 20. JWT auth with groupId required. Redis 7 and Postgres 16 available.
- Kernel config now includes signing/tenant enforcement; add env `OPENAI_API_KEY`, optional `OPENAI_API_BASE`, `OPENAI_DEFAULT_MODEL`.
- Rate-limit library: @fastify/rate-limit already in use; reuse or add simple middleware.

Requirements
1) Endpoint
   - `POST /ai/chat`
   - Body: { model?: string, messages: [{role, content}], stream?: boolean, temperature?, max_tokens? }
   - Defaults: model from env, stream=false.

2) Behavior
   - Validate input with zod.
   - Call OpenAI API using server-held key; optionally allow `OPENAI_API_BASE`.
   - Record usage (prompt_tokens, completion_tokens, model, groupId, userId) to AuditLog or a new table (choose simplest).
   - Enforce rate limit per groupId (and per userId). Start with a simple in-memory or Redis token bucket (e.g., 60 req/5min).

3) Streaming
   - If stream=true, respond with SSE (text/event-stream) or keep it non-stream for first pass; pick one and document.
   - Client must include Authorization Bearer as usual; no client API key.

4) Security
   - Reject if groupId missing. No proxying arbitrary URLs (only OpenAI).
   - Do not log prompts verbatim in production logs; audit can store token counts and model.

5) Tests
   - Non-stream happy path with mocked OpenAI fetch.
   - Rate-limit rejection.
   - groupId missing => 401.

Deliverables
   - Route file (e.g., `src/api/routes/ai.ts`), config additions, env template updates.
   - Tests covering validation, auth, and mocked completion.
