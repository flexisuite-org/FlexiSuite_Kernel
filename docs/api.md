# API Summary

- Base: HTTP Fastify v4
- Health: `GET /health` -> { status, db, redis }
- Metrics: `GET /metrics` (Prometheus)
- Auth:
  - `POST /auth/signup` { email, password } -> tokens
  - `POST /auth/login` { email, password } -> tokens
  - `POST /auth/refresh` { userId, refreshToken, familyId? } -> tokens (rotation + reuse detection)
- Hooks/middleware: context plugin sets current group/user for Prisma; RBAC middleware skeleton.
- Planned: entity CRUD, event publishing, sandbox exec endpoints.
