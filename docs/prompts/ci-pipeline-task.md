# Prompt: Add CI Pipeline for FlexiSuite Kernel

Goal
- Add a GitHub Actions workflow that installs deps, brings up Postgres/Redis services, and runs `pnpm test` (runInBand, no forceExit).

Context
- Node 20, pnpm 10.x. Postgres 16 on port 5433, Redis 7 on port 6380. Uses Prisma 5.9.
- Env: SIGNING_SECRET can be a test default; DATABASE_URL / REDIS_URL must point to the services.

Requirements
1) Workflow
   - Trigger: push/PR to main.
   - Steps: checkout, setup-node (20.x), setup-pnpm, cache pnpm store, install, start Postgres & Redis services, wait for readiness, run `pnpm test`.
   - Services: use `postgres:16` (port 5433), `redis:7` (port 6380).

2) Env
   - DATABASE_URL=postgresql://flexi:flexi@localhost:5433/flexi?schema=public
   - REDIS_URL=redis://localhost:6380
   - JWT_SECRET/REFRESH_TOKEN_SECRET/SIGNING_SECRET: set dummy test values.

3) Notes
   - No build artifact needed.
   - prisma migrate deploy before tests (if required) or rely on truncateAll seeding; choose consistent approach (recommend `pnpm prisma migrate deploy`).

Deliverables
   - `.github/workflows/ci.yml` implementing the above.
