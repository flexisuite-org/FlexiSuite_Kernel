# Tech Debt: Implement Real Signature Verification in RLS

**Severity**: High (Security)
**Component**: `kernel-data` / `PostgreSQL Migration`
**Created**: 2026-02-16

## Description
The current implementation of `flexi.authorize_tenant()` in migration `m20240216_000001` relies on a mocked signature verification.

```sql
-- 5. Verify Signature (Mock for now, REAL IMPLEMENTATION needed via pgcrypto/plrust)
-- ...
-- To match the Rust mock: "v2:mock_signature:{tenant}:{nonce}" we check strict equality for now.
```

This allows any client with DB access to forge a tenant token if they know the format. While `kernel-api` is the only trusted client today, this violates the Defense in Depth principle.

## Requirements
1.  **Crypto Extension**: Enable `pgcrypto` or implement a `plrust` function to verify Ed25519 signatures.
2.  **Key Management**: The database needs access to the Public Key (Tenant Verification Key) to verify the signature `v2:kid:ts:nonce:tenant_id`.
3.  **Migration**: Update `flexi.authorize_tenant()` to perform actual verification.

## Success Criteria
- [ ] Attempting to authorize with a modified signature MUST fail.
- [ ] Attempting to authorize with an expired timestamp MUST fail (already implemented but re-verify).
- [ ] `flexi_nonce` table MUST prevent replay attacks (already implemented).
