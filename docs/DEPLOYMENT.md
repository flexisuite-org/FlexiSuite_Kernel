# Deployment Guide (Fastify 5 / Ports: app 9000, PG 5433, Redis 6380)

## Infra
- Node 20.x, pnpm 10.x
- Postgres 16 (listen on 5433) / Redis 7 (6380)
- Nginx (TLS終端), PM2

## Quick Steps (VPS例)
```bash
# Install basics
sudo apt update && sudo apt upgrade -y
sudo apt install -y ca-certificates curl gnupg nginx redis-server
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt install -y nodejs
npm install -g pnpm pm2

# Postgres 16
sudo apt install -y postgresql-16
sudo -u postgres psql -c "CREATE USER flexi WITH PASSWORD '...';"
sudo -u postgres psql -c "CREATE DATABASE flexi OWNER flexi;"
```

## Env (.env prod 例)
```
NODE_ENV=production
PORT=9000
DATABASE_URL=postgresql://flexi:PASSWORD@localhost:5433/flexi
REDIS_URL=redis://:PASSWORD@localhost:6380
JWT_SECRET=<random-256-bit>
REFRESH_TOKEN_SECRET=<random-256-bit>
RATE_LIMIT_MAX=100
RATE_LIMIT_WINDOW=60000
SANDBOX_MEMORY_MB=128
SANDBOX_TIMEOUT_MS=500
LOG_LEVEL=info
```

## App Deploy
```bash
git pull
pnpm install --prod
pnpm prisma generate
pnpm prisma migrate deploy
pnpm build
pm2 start dist/index.js --name flexisuite-kernel --env production
pm2 save
```

## RLS 適用
```
psql -h localhost -p 5433 -U flexi -d flexi -f prisma/rls.sql
```

## Nginx (例)
- upstream: 127.0.0.1:9000
- /health は公開、/metrics は内部ネットワークのみ許可
- TLS: certbot --nginx -d api.example.com

## Monitoring
- /health, /metrics を Prometheus でスクレイプ
- PM2 logs / status

## Backup/Restore (概要)
- Backup: `pg_dump -h localhost -p 5433 -U flexi -Fc flexi > backup.dump`
- Restore: `pg_restore -h localhost -p 5433 -U flexi -d flexi backup.dump`
- 定期実行は cron、暗号化 (age/GPG) 推奨

## Checklist
- [ ] env 設定済み（上記ポートに合わせる）
- [ ] prisma migrate deploy 済み
- [ ] rls.sql 適用済み（テナント境界必要な場合）
- [ ] pm2 start / nginx reload 完了
- [ ] /health OK, /metrics OK
