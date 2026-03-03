#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};
    use kernel_core::supplychain::{
        BreakGlassContext, KeyStatus, Manifest, TrustedKey, VerificationResult,
        manifest_signing_payload, signature_scheme_for_digest, verify_break_glass, verify_manifest,
    };

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    const DIGEST_SHA256_REVOKED: &str =
        "sha256-1111111111111111111111111111111111111111111111111111111111111111";
    const DIGEST_SHA256_ACTIVE: &str =
        "sha256-2222222222222222222222222222222222222222222222222222222222222222";
    const DIGEST_SHA384_ACTIVE: &str = "sha384-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DIGEST_SHA384_SCOPE: &str = "sha384-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn sign_manifest(manifest: &Manifest, signing_key: &SigningKey) -> String {
        let scheme = signature_scheme_for_digest(&manifest.digest)
            .expect("unsupported digest format in test manifest");
        let sig = signing_key.sign(&manifest_signing_payload(manifest, scheme));
        hex::encode(sig.to_bytes())
    }

    #[test]
    fn test_manifest_signature_trust_root() {
        let signing_key = signing_key();
        let public_key = signing_key.verifying_key().to_bytes().to_vec();

        let manifest_revoked = Manifest {
            id: "pkg-a".to_string(),
            digest: DIGEST_SHA256_REVOKED.to_string(),
            signature: String::new(),
            kid: "revoked".to_string(),
        };
        let manifest_revoked = Manifest {
            signature: sign_manifest(&manifest_revoked, &signing_key),
            ..manifest_revoked
        };

        let manifest_ok = Manifest {
            id: "pkg-b".to_string(),
            digest: DIGEST_SHA256_ACTIVE.to_string(),
            signature: String::new(),
            kid: "active".to_string(),
        };
        let manifest_ok = Manifest {
            signature: sign_manifest(&manifest_ok, &signing_key),
            ..manifest_ok
        };

        // Mock time
        let now = 100000;
        let retired_at_ok = now - 50; // 50s ago (within 86400s)

        let retired_at_fail = now - 90000; // 90000s ago (> 86400)

        // Trusted Keys
        let key_active = TrustedKey {
            kid: "active".to_string(),
            public_key: public_key.clone(),
            status: KeyStatus::Active,
            retired_at: None,
        };
        let key_revoked = TrustedKey {
            kid: "revoked".to_string(),
            public_key: public_key.clone(),
            status: KeyStatus::Revoked,
            retired_at: None,
        };
        let key_retired_ok = TrustedKey {
            kid: "active".to_string(),
            public_key: public_key.clone(),
            status: KeyStatus::Retired,
            retired_at: Some(retired_at_ok),
        };
        let key_retired_fail = TrustedKey {
            kid: "active".to_string(),
            public_key: public_key.clone(),
            status: KeyStatus::Retired,
            retired_at: Some(retired_at_fail),
        };
        let key_next = TrustedKey {
            kid: "active".to_string(),
            public_key: public_key.clone(),
            status: KeyStatus::Next,
            retired_at: None,
        };

        assert!(matches!(
            verify_manifest(&manifest_revoked, &key_revoked, DIGEST_SHA256_REVOKED, now),
            VerificationResult::KeyRevoked
        ));

        // Test Digest Mismatch (Contract: Mandatory check)
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_active, "sha256-WRONG", now),
            VerificationResult::DigestMismatch
        ));
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_active, DIGEST_SHA256_ACTIVE, now),
            VerificationResult::Ok
        ));

        let manifest_sha384 = Manifest {
            id: "pkg-c".to_string(),
            digest: DIGEST_SHA384_ACTIVE.to_string(),
            signature: String::new(),
            kid: "active".to_string(),
        };
        let manifest_sha384 = Manifest {
            signature: sign_manifest(&manifest_sha384, &signing_key),
            ..manifest_sha384
        };

        // Verifying dash prefix support
        assert!(matches!(
            verify_manifest(&manifest_sha384, &key_active, DIGEST_SHA384_ACTIVE, now),
            VerificationResult::Ok
        ));

        // Test Key Mismatch
        let key_wrong = TrustedKey {
            kid: "wrong".to_string(),
            public_key: public_key.clone(),
            status: KeyStatus::Active,
            retired_at: None,
        };
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_wrong, DIGEST_SHA256_ACTIVE, now),
            VerificationResult::KeyMismatch
        ));

        // Test Retired logic
        // 1. In Window -> OK
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_retired_ok, DIGEST_SHA256_ACTIVE, now),
            VerificationResult::Ok
        ));

        // 2. Out Window -> Fail
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_retired_fail, DIGEST_SHA256_ACTIVE, now),
            VerificationResult::KeyRetiredOutOfWindow
        ));

        // 3. Next -> OK
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_next, DIGEST_SHA256_ACTIVE, now),
            VerificationResult::Ok
        ));
    }

    #[test]
    fn test_manifest_signature_invalid_path() {
        let signing_key = signing_key();
        let public_key = signing_key.verifying_key().to_bytes().to_vec();

        let manifest_ok = Manifest {
            id: "pkg-sig-invalid".to_string(),
            digest: DIGEST_SHA256_ACTIVE.to_string(),
            signature: String::new(),
            kid: "active".to_string(),
        };
        let manifest_ok = Manifest {
            signature: sign_manifest(&manifest_ok, &signing_key),
            ..manifest_ok
        };

        let mut manifest_bad_sig = manifest_ok.clone();
        manifest_bad_sig.signature = "00".repeat(64);

        let trusted_key = TrustedKey {
            kid: "active".to_string(),
            public_key,
            status: KeyStatus::Active,
            retired_at: None,
        };

        assert!(matches!(
            verify_manifest(
                &manifest_bad_sig,
                &trusted_key,
                DIGEST_SHA256_ACTIVE,
                100000
            ),
            VerificationResult::SignatureInvalid
        ));
    }

    #[test]
    fn test_manifest_break_glass_scope_and_ttl() {
        let ctx = BreakGlassContext {
            enabled: true,
            scope_tenant_id: Some("tenant_A".to_string()),
            scope_digest: Some(DIGEST_SHA384_SCOPE.to_string()),
            expiry_ts: 100, // timestamp
        };

        // Case 1: Matching Scope -> ALLOW
        assert!(matches!(
            verify_break_glass(&ctx, "tenant_A", DIGEST_SHA384_SCOPE, 50),
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
            verify_break_glass(&ctx_disabled, "tenant_A", DIGEST_SHA384_SCOPE, 50),
            VerificationResult::BreakGlassDisabled
        ));

        // Case 2: Mismatch Scope -> BLOCK
        assert!(matches!(
            verify_break_glass(&ctx, "tenant_B", DIGEST_SHA384_SCOPE, 50),
            VerificationResult::BreakGlassScopeMismatch
        ));

        // Case 3: Expired strictness
        // Expiry = 100. Now = 100 -> Expired.
        assert!(matches!(
            verify_break_glass(&ctx, "tenant_A", DIGEST_SHA384_SCOPE, 100),
            VerificationResult::BreakGlassExpired
        ));

        // Case 5: Missing Scope -> BLOCK (Global bypass attempt)
        let ctx_global = BreakGlassContext {
            enabled: true,
            scope_tenant_id: None, // Missing
            scope_digest: Some(DIGEST_SHA384_SCOPE.to_string()),
            expiry_ts: 200,
        };
        assert!(matches!(
            verify_break_glass(&ctx_global, "tenant_A", DIGEST_SHA384_SCOPE, 50),
            VerificationResult::BreakGlassScopeMissing
        ));
    }
}
