# Dev Workflow

Setup:
- pnpm install
- docker compose up -d postgres redis
- pnpm prisma migrate dev --name init
- pnpm prisma generate

Daily:
- pnpm dev
- curl http://localhost:9000/health

Testing (placeholder):
- pnpm test  # jest placeholder; expand with integration tests later

Coding notes:
- Keep schema changes via Prisma migrate; rerun generate after edits.
- RLS: set flexi.current_group via context plugin; ensure tests cover cross-tenant denial.
