# FlexiSuite Kernel Implementation Plan

# Goal Description
Initialize the **FlexiSuite Kernel** (v3.1 Spec), the OS-like backend for the FlexiSuite SaaS.
This plan covers the foundational setup, core modules (IAM, Data, Events), and operational baselines.

## User Review Required
> [!IMPORTANT]
> **Security**:
> - **Passwords**: Argon2id.
> - **Sessions**: JWT (15m) + Refresh Token (7d) with Rotation.
> - **Tenancy**: Prisma Middleware enforces `groupId`.
> **Infrastructure**:
> - **Local Dev**: `docker-compose` (Postgres + Redis).
> - **Production**: VPS with PM2.

## Proposed Changes

### 1. Project Foundation
#### [NEW] [package.json](file:///home/yohaku/FlexiSuite_Kernel/package.json)
- **Core**: `fastify`, `prisma`, `@prisma/client`.
- **Auth**: `argon2`, `jsonwebtoken`.
- **Ops**: `pino`, `dotenv`, `zod`.
- **Runtime**: `isolated-vm`, `bullmq`.
- **Security**: `@fastify/helmet`, `@fastify/rate-limit`, `@fastify/cors`.
- **Dev**: `typescript`, `ts-node`, `jest`, `supertest`.

#### [NEW] [docker-compose.yml](file:///home/yohaku/FlexiSuite_Kernel/docker-compose.yml)
- PostgreSQL (v16)
- Redis (v7)

#### [NEW] [.env.example](file:///home/yohaku/FlexiSuite_Kernel/.env.example)
- Template for environment variables (`DATABASE_URL`, `REDIS_URL`, `JWT_SECRET`, etc.).

### 2. Core Modules Implementation

#### [NEW] [src/kernel/iam/](file:///home/yohaku/FlexiSuite_Kernel/src/kernel/iam/)
- `schema.prisma`: Define `User`, `Group`, `GroupMember`, `Role`, `Permission`, `RolePermission`, `RefreshToken`, `AuditLog`.
- `auth.service.ts`: Signup, Login, **Refresh Token Rotation + Reuse Detection**.
- `context.plugin.ts`: Sets `flexi.current_group` / `flexi.current_user` for RLS.
- `middleware.ts`: **Prisma Middleware** for tenancy.

#### [NEW] [src/kernel/data/](file:///home/yohaku/FlexiSuite_Kernel/src/kernel/data/)
- `schema.prisma`: Define `EntityDefinition`, `EntityRecord`, `EntityHistory`.
- `validator.ts`: JSON Schema validation (Ajv).
- `repository.ts`: CRUD with Lazy Migration logic and enforced `groupId` scoping.
- `migration.ts`: Helpers for background backfill to latest schema_version.

#### [NEW] [src/kernel/events/](file:///home/yohaku/FlexiSuite_Kernel/src/kernel/events/)
- `bus.ts`: **BullMQ** setup with At-least-once delivery, retries, and DLQ.
- `definitions.ts`: Zod schemas for events.

#### [NEW] [src/kernel/runtime/](file:///home/yohaku/FlexiSuite_Kernel/src/kernel/runtime/)
- `sandbox.ts`: `isolated-vm` wrapper.
- `policy.ts`: Module allowlist, memory/time quotas, and scrubbed logs.

### 3. API & Operations

#### [NEW] [src/api/](file:///home/yohaku/FlexiSuite_Kernel/src/api/)
- `server.ts`: Fastify setup with **Helmet**, **RateLimit** (100/min), **Cors**.
- `health.ts`: `/health` endpoint.
- `auth.routes.ts`: Signup/Login/Refresh endpoints with per-IP throttle.
- `metrics.ts`: `/metrics` endpoint.
- `hooks/context.ts`: request context -> Prisma RLS.

## Verification Plan

### Automated Tests
- **Unit Tests**: Test Password Hashing, Token Rotation, Schema Validation.
- **Integration Tests**:
    - Verify `groupId` isolation (try to access other group's data).
    - Verify Rate Limiting.
    - Refresh token reuse detection.
    - Event idempotency (duplicate event processed once).

### Manual Verification
- **Health Check**: `curl http://localhost:3000/health`.
- **Audit Log**: Perform action and check `AuditLog` table.
- **Backup Drill**: `pg_dump` -> restore into temp DB -> run smoke queries.
