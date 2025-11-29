#!/usr/bin/env bash
set -euo pipefail

# Start local dependencies and app for development.

echo "[1/4] Starting postgres+redis..."
docker compose up -d postgres redis

if ! pnpm prisma show >/dev/null 2>&1; then
  echo "[2/4] Running prisma migrate (init)..."
  pnpm prisma migrate dev --name init
fi

echo "[3/4] Generating Prisma client..."
pnpm prisma generate

echo "[4/4] Starting app..."
pnpm dev
