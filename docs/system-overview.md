# System Overview

- Purpose: FlexiSuite Kernel backend (multi-tenant SaaS OS layer) providing auth/RBAC, event bus, meta-schema data engine, sandbox runtime.
- Stack: Node.js + TypeScript, Fastify v4, PostgreSQL (Prisma), Redis, BullMQ, isolated-vm, Pino, Zod.
- Runtime: Fastify server on configurable port (default 9000), JSON logging, env validated via Zod.
- Modules:
  - IAM: Argon2id passwords, JWT 15m, rotating refresh (7d) with reuse detection, RBAC via Role/Permission, Prisma middleware + RLS.
  - Data Engine: EntityDefinition (JSON Schema) + EntityRecord (JSONB, schemaVersion), lazy migration + optional backfill.
  - Events: EventEmitter + BullMQ (at-least-once, DLQ/retry planned).
  - Runtime: isolated-vm sandbox (128MB/500ms, no network by default).
- Observability: /health, /metrics, Pino logs.
- Security posture: Rate limiting (@fastify/rate-limit v8), CSRF token for refresh, audit logging.
