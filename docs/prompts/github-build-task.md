# Prompt: Implement GitHub Build Automation for FlexiSuite Kernel

Goal
- Implement the GitHub integration workflow so that a request to `/integrations/github/build` can clone a repo, build it, produce a ZIP artifact, upload it to the Kernel registry (bundleUpload), and report status (via API + WebSocket).

Context
- Kernel API base: internal Fastify server (Node 20, Fastify 5, Prisma 5.9, PostgreSQL 16 on 5433, Redis 7 on 6380).
- Auth: JWT Bearer; groupId is required (RLS). Requests come with Authorization: Bearer token.
- Existing endpoints:
  - `POST /integrations/github/build` (stub) — should enqueue a job and return {jobId, status:"queued"}.
  - `GET /integrations/github/status?jobId=...` (stub) — should return job status/progress.
  - `POST /integrations/github/webhook` — receives GitHub events (signature optional, secret in GITHUB_WEBHOOK_SECRET).
  - Registry/upload: `POST /registry/packages/:id/bundleUpload`
  - Approve: `POST /registry/packages/:id/approve`
  - Install: `POST /install`
- WebSocket: `/ws` exists; JWT required. Use it to push progress events (channel by jobId).

Requirements
1) Job handling
   - Use BullMQ (already in deps) or a simple in-process queue to avoid blocking the API.
   - Job data: { jobId, repo, branch, buildCommand, artifactPath, packageName, version, groupId, userId }.
   - Persist minimal state in DB (AuditLog) and/or Redis so status survives process restarts.

2) Build runner
   - Clone repo (with optional GITHUB_TOKEN for private repos).
   - Checkout branch (default main).
   - Run buildCommand (default `npm ci && npm run build`).
   - Locate or create ZIP artifact at artifactPath. If not zipped, zip it.
   - Call `bundleUpload` for the target packageId (derive from packageName/version; create package if not exists under ownerGroupId).
   - Optionally approve/install if flags are provided (you may add fields approve=true, install=true).

3) Status & events
   - Update job status: queued -> cloning -> building -> bundling -> uploading -> done/failed.
   - Emit events over WebSocket channel `job:<jobId>` with {status, message, step, progress?}.
   - `/integrations/github/status` should return current status and last error if failed.

4) Security
   - JWT required; use groupId from token for all DB operations and package ownership.
   - Webhook signature: if GITHUB_WEBHOOK_SECRET is set, verify `x-hub-signature-256`.
   - Never expose secrets to clients; only status text and progress.

5) Limits/assumptions
   - Artifact size modest (<50MB); streaming upload not required.
   - Single worker process is acceptable; design so it can be moved to BullMQ later if needed.

Deliverables
   - Implement endpoints in `src/api/routes/github.ts` plus worker/queue modules.
   - Status retrieval and WS publish helper.
   - Tests: minimal happy-path test (mock repo/zip) and signature verification test.
