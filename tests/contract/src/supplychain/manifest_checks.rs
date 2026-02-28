#[cfg(test)]
mod tests {
    use kernel_core::supplychain::{
        BreakGlassContext, VerificationResult, verify_break_glass,
    };
    #[cfg(feature = "test-utils")]
    use kernel_core::supplychain::{
        KeyStatus, Manifest, TrustedKey, verify_manifest,
    };

    #[test]
    #[cfg(feature = "test-utils")]
    fn test_manifest_signature_trust_root() {
        let manifest_revoked = Manifest {
            id: "pkg-a".to_string(),
            digest: "sha256-123".to_string(),
            signature: "sig".to_string(),
            kid: "revoked".to_string(),
        };

        let manifest_ok = Manifest {
            id: "pkg-b".to_string(),
            digest: "sha256-456".to_string(),
            signature: "sig".to_string(),
            kid: "active".to_string(),
        };

        // Mock time
        let now = 100000;
        let retired_at_ok = now - 50; // 50s ago (within 86400s)

        let retired_at_fail = now - 90000; // 90000s ago (> 86400)

        // Trusted Keys
        let key_active = TrustedKey {
            kid: "active".to_string(),
            status: KeyStatus::Active,
            retired_at: None,
            public_key: [0u8; 32],
        };
        let key_revoked = TrustedKey {
            kid: "revoked".to_string(),
            status: KeyStatus::Revoked,
            retired_at: None,
            public_key: [0u8; 32],
        };
        let key_retired_ok = TrustedKey {
            kid: "active".to_string(),
            status: KeyStatus::Retired,
            retired_at: Some(retired_at_ok),
            public_key: [0u8; 32],
        };
        let key_retired_fail = TrustedKey {
            kid: "active".to_string(),
            status: KeyStatus::Retired,
            retired_at: Some(retired_at_fail),
            public_key: [0u8; 32],
        };
        let key_next = TrustedKey {
            kid: "active".to_string(),
            status: KeyStatus::Next,
            retired_at: None,
            public_key: [0u8; 32],
        };

        assert!(matches!(
            verify_manifest(&manifest_revoked, &key_revoked, "sha256-123", now),
            VerificationResult::KeyRevoked
        ));

        // Test Digest Mismatch (Contract: Mandatory check)
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_active, "sha256-WRONG", now),
            VerificationResult::DigestMismatch
        ));
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_active, "sha256-456", now),
            VerificationResult::Ok
        ));

        let manifest_sha384 = Manifest {
            id: "pkg-c".to_string(),
            digest: "sha384-abc".to_string(),
            signature: "sig".to_string(),
            kid: "active".to_string(),
        };

        // Verifying dash prefix support
        assert!(matches!(
            verify_manifest(&manifest_sha384, &key_active, "sha384-abc", now),
            VerificationResult::Ok
        ));

        // Test Key Mismatch
        let key_wrong = TrustedKey {
            kid: "wrong".to_string(),
            status: KeyStatus::Active,
            retired_at: None,
            public_key: [0u8; 32],
        };
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_wrong, "sha256-456", now),
            VerificationResult::KeyMismatch
        ));

        // Test Retired logic
        // 1. In Window -> OK
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_retired_ok, "sha256-456", now),
            VerificationResult::Ok
        ));

        // 2. Out Window -> Fail
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_retired_fail, "sha256-456", now),
            VerificationResult::KeyRetiredOutOfWindow
        ));

        // 3. Next -> OK
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_next, "sha256-456", now),
            VerificationResult::Ok
        ));
    }

    #[test]
    fn test_manifest_break_glass_scope_and_ttl() {
        let ctx = BreakGlassContext {
            enabled: true,
            scope_tenant_id: Some("tenant_A".to_string()),
            scope_digest: Some("sha384-xyz".to_string()),
            expiry_ts: 100, // timestamp
        };

        // Case 1: Matching Scope -> ALLOW
        assert!(matches!(
            verify_break_glass(&ctx, "tenant_A", "sha384-xyz", 50),
            VerificationResult::Ok
        ));

        // Case 1b: Disabled -> BreakGlassDisabled
        let ctx_disabled = BreakGlassContext {
            enabled: false,
            scope_tenant_id: ctx.scope_tenant_id.clone(),
            scope_digest: ctx.scope_digest.clone(),
            expiry_ts: ctx.expiry_ts,
        };
        assert!(matches!(
            verify_break_glass(&ctx_disabled, "tenant_A", "sha384-xyz", 50),
            VerificationResult::BreakGlassDisabled
        ));

        // Case 2: Mismatch Scope -> BLOCK
        assert!(matches!(
            verify_break_glass(&ctx, "tenant_B", "sha384-xyz", 50),
            VerificationResult::BreakGlassScopeMismatch
        ));

        // Case 3: Expired strictness
        // Expiry = 100. Now = 100 -> Expired.
        assert!(matches!(
            verify_break_glass(&ctx, "tenant_A", "sha384-xyz", 100),
            VerificationResult::BreakGlassExpired
        ));

        // Case 5: Missing Scope -> BLOCK (Global bypass attempt)
        let ctx_global = BreakGlassContext {
            enabled: true,
            scope_tenant_id: None, // Missing
            scope_digest: Some("sha384-xyz".to_string()),
            expiry_ts: 200,
        };
        assert!(matches!(
            verify_break_glass(&ctx_global, "tenant_A", "sha384-xyz", 50),
            VerificationResult::BreakGlassScopeMissing
        ));
    }
}
