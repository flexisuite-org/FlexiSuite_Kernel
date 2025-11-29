# Ops (Runbook)

Env vars (see .env.example): DATABASE_URL, REDIS_URL, JWT_SECRET, REFRESH_TOKEN_SECRET, PORT, RATE_LIMIT_*, SANDBOX_*, LOG_LEVEL.

Start (local dev):
1) docker compose up -d postgres redis
2) pnpm prisma migrate dev --name init
3) psql -h localhost -p 5433 -U flexi -d flexi -f prisma/rls.sql   # optional if RLS使う
4) pnpm dev

Health/Observability:
- /health, /metrics, Pino JSON logs.

Backup/Restore (manual):
- Backup: pg_dump -h localhost -p 5433 -U flexi -Fc flexi > backup.dump
- Restore: pg_restore -h localhost -p 5433 -U flexi -d flexi backup.dump

PM2 + Nginx (prod outline):
- Build: pnpm build; pm2 start dist/index.js --name flexi-kernel
- Nginx: reverse proxy 80/443 -> 9000, with TLS (Let’s Encrypt) and /health check.
