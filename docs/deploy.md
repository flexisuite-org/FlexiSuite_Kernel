# Deploy Outline

Build & artifacts:
- pnpm build (TS -> dist)

Runtime:
- PORT (default 9000), DATABASE_URL, REDIS_URL, JWT/REFRESH secrets.
- Start: pm2 start dist/index.js --name flexi-kernel --env production

Reverse proxy:
- Nginx terminates TLS; proxy_pass to http://127.0.0.1:9000; add /health for upstream check.

Data:
- Postgres (managed or self-hosted), Redis (managed or self-hosted). Configure connection strings accordingly.

Rollout/Rollback:
- Use pm2 reload for zero-downtime; rollback by redeploying previous dist + env.

Monitoring:
- Scrape /metrics (Prometheus); alert on p95 latency, error rate, queue lag, DB/Redis availability.

Backup/DR:
- pg_dump scheduled; periodic restore test into staging.
