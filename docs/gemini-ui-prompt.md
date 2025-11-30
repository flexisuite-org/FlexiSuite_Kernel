You are designing a light-themed admin/marketplace UI that talks **only** to the FlexiSuite Kernel REST API (no direct DB/S3 access). Layout is up to you—be bold/expressive—but keep it clear.

Must-have features (feel free to arrange/reimagine screens):
- Auth: email/password login (`POST /auth/login`), store JWT and send as Bearer.
- Health: show `/health` status (db/redis).
- Registry: list packages (`GET /registry/packages`), filter by status; view manifest, bundleIntegrity/signature; actions approve/revoke/upload (bundleUpload sends base64).
- GitHub panel: fields for repo URL, branch, build command, artifact path, webhook secret; buttons for “copy webhook URL” (`/integrations/github/webhook`), “trigger build” (`POST /integrations/github/build`), show latest status (`GET /integrations/github/status`). Webhook handling is server-side; UI just configures and shows status.
- Install: list installs (`GET /install`), create install (`POST /install`).
- Run tester: pick install, enter JSON payload, optional draft toggle (header `x-flexi-mode: draft`), call `POST /components/:id/run`, show JSON results.
- WebSocket: connect to `/ws` with JWT (echo now; future build/log streaming). Provide a minimal console view.

Constraints / rules:
- Base URL from env `NEXT_PUBLIC_KERNEL_API`.
- Bearer JWT on all calls; no extra cookies/sessions.
- All uploads go through `/registry/packages/:id/bundleUpload` (ZIP recommended).
- Group isolation: JWT must carry `groupId`; show the active group somewhere.
- Capability allowlist/role allowlist enforced server-side; just surface errors.

Deliverables:
- Page/component structure, states (loading/error/empty).
- Color/typography system for a light theme (1 accent color).
- Interaction details for bundle upload (file→base64), run tester, GitHub trigger/status, WS console.
- Keep copy minimal; emphasize clarity and admin confidence.
