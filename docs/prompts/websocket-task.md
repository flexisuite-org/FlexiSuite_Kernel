# Prompt: Enhance WebSocket Support for FlexiSuite Kernel

Goal
- Provide authenticated WebSocket channels to stream build progress and other events.

Context
- Stack: Fastify 5, Node 20, `@fastify/websocket` already registered under `/ws`.
- Auth: JWT Bearer with groupId required. ALS is used elsewhere; here we validate token on connection.
- Redis 7 available on 6380 (ioredis). Single-instance acceptable, but design an abstraction that could swap in Redis PubSub later.
- Existing `/ws` now just echoes.

Requirements
1) Auth handshake
   - Accept Authorization: Bearer token header (or Sec-WebSocket-Protocol if needed).
   - Decode JWT with config.JWT_SECRET; reject if missing/invalid or groupId absent.
   - Store {userId, groupId, roles} in connection context.

2) Channel model
   - Support publish/subscribe by string channel (e.g., `job:<jobId>`).
   - Provide server helper `publishWs(channel, payload)`.
   - Clients subscribe by sending `{type:"subscribe", channel:"job:<id>"}`; unsubscribe likewise.
   - Broadcast only to connections in same groupId (enforce tenancy).

3) Usage: Build progress
   - GitHub build worker will publish to `job:<jobId>`. WS should deliver messages like `{status, message, step}` to subscribed clients.

4) Connection lifecycle
   - On close, remove subscriptions.
   - Heartbeat/ping every 30s to keep connections alive (optional).

5) Tests
   - Fails without JWT.
   - Subscribes then receives a publish to that channel.
   - Cross-group isolation: publish to channel with group A should not reach group B connection.

Deliverables
   - WS route implementation (replace current echo) in `src/api/routes/ws.ts` or helpers.
   - Publish helper and in-memory (or Redis-backed) registry of subscriptions.
   - Minimal tests (superwstest or ws) to cover auth and subscription flow.
