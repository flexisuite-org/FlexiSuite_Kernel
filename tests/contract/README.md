# Contract Test Scaffolding

このディレクトリは `docs/verification_matrix.md` に対応する契約テストの雛形です。

- `auth/`: 認証/RLS/tenant_token
- `quota/`: HTTP 429/503 と `Retry-After`
- `idempotency/`: `Idempotency-Key` と `X-Action-Id`
- `worker/`: protocol fallback / a11y / canvas fallback metrics
- `diagnostics/`: PII scrub / consent
- `supplychain/`: trust root / manifest署名 / break-glass
- `slo/`: SLO smoke / reproducibility
