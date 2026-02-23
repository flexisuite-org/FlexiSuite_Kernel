#[cfg(test)]
mod tests {
    // SigningKey/Signer/OsRng are from ed25519-dalek, while production verification uses ring.
    // Tests MUST use plain RFC 8032 Ed25519 (no Ed25519ph, no context): the exact bytes passed to
    // SigningKey.sign(...) must be identical to the bytes verify_manifest checks as expected_digest.
    use ed25519_dalek::{Signer, SigningKey};
    use kernel_core::supplychain::{
        BreakGlassContext, KeyStatus, Manifest, TrustedKey, VerificationResult, verify_break_glass,
        verify_manifest,
    };
    use rand::rngs::OsRng;

    #[test]
    fn test_manifest_signature_trust_root() {
        let mut csprng = OsRng;
        let signing_key_active = SigningKey::generate(&mut csprng);
        let verifying_key_active = signing_key_active.verifying_key();
        let pub_key_active = hex::encode(verifying_key_active.to_bytes());

        let signing_key_revoked = SigningKey::generate(&mut csprng);
        let verifying_key_revoked = signing_key_revoked.verifying_key();
        let pub_key_revoked = hex::encode(verifying_key_revoked.to_bytes());

        // Digest to sign
        let digest_123 = "sha256-123";
        let _signature_123_active =
            hex::encode(signing_key_active.sign(digest_123.as_bytes()).to_bytes());
        let signature_123_revoked =
            hex::encode(signing_key_revoked.sign(digest_123.as_bytes()).to_bytes());

        let manifest_revoked = Manifest {
            id: "pkg-a".to_string(),
            digest: digest_123.to_string(),
            signature: signature_123_revoked,
            kid: "revoked".to_string(),
        };

        let digest_456 = "sha256-456";
        let digest_456_bytes = digest_456.as_bytes();
        let signature_456_active_bytes = signing_key_active.sign(digest_456_bytes).to_bytes();
        assert_eq!(
            signature_456_active_bytes.len(),
            64,
            "Ed25519 signatures must be exactly 64 bytes (RFC 8032)"
        );
        let signature_456_active = hex::encode(signature_456_active_bytes);
        let manifest_ok = Manifest {
            id: "pkg-b".to_string(),
            digest: digest_456.to_string(),
            signature: signature_456_active,
            kid: "active".to_string(),
        };

        // Mock time
        let now = 100000;
        let retired_at_ok = now - 50; // 50s ago (within 86400s)
        let retired_at_fail = now - 90000; // 90000s ago (> 86400)

        // Trusted Keys
        let key_active = TrustedKey {
            kid: "active".to_string(),
            alg: "Ed25519".to_string(),
            public_key: pub_key_active.clone(),
            status: KeyStatus::Active,
            retired_at: None,
            not_before: None,
            not_after: None,
        };
        let key_revoked = TrustedKey {
            kid: "revoked".to_string(),
            alg: "Ed25519".to_string(),
            public_key: pub_key_revoked.clone(),
            status: KeyStatus::Revoked,
            retired_at: None,
            not_before: None,
            not_after: None,
        };
        let key_retired_ok = TrustedKey {
            kid: "active".to_string(),
            alg: "Ed25519".to_string(),
            public_key: pub_key_active.clone(), // Same key but retired
            status: KeyStatus::Retired,
            retired_at: Some(retired_at_ok),
            not_before: None,
            not_after: None,
        };
        let key_retired_fail = TrustedKey {
            kid: "active".to_string(),
            alg: "Ed25519".to_string(),
            public_key: pub_key_active.clone(),
            status: KeyStatus::Retired,
            retired_at: Some(retired_at_fail),
            not_before: None,
            not_after: None,
        };
        let key_retired_none = TrustedKey {
            kid: "active".to_string(),
            alg: "Ed25519".to_string(),
            public_key: pub_key_active.clone(),
            status: KeyStatus::Retired,
            retired_at: None,
            not_before: None,
            not_after: None,
        };
        let key_next = TrustedKey {
            kid: "active".to_string(),
            alg: "Ed25519".to_string(),
            public_key: pub_key_active.clone(),
            status: KeyStatus::Next,
            retired_at: None,
            not_before: None,
            not_after: None,
        };
        let key_wrong_alg = TrustedKey {
            kid: "active".to_string(),
            alg: "RS256".to_string(),
            public_key: pub_key_active.clone(),
            status: KeyStatus::Active,
            retired_at: None,
            not_before: None,
            not_after: None,
        };

        // Revoked Key -> KeyRevoked
        assert!(matches!(
            verify_manifest(
                "tenant_test",
                &manifest_revoked,
                &key_revoked,
                digest_123,
                now
            ),
            VerificationResult::KeyRevoked
        ));

        // Test Digest Mismatch (Contract: Mandatory check)
        // Even if signature is valid for "sha256-456", if expected digest is different, fail digest match.
        // manifest_ok has digest "sha256-456".
        assert!(matches!(
            verify_manifest(
                "tenant_test",
                &manifest_ok,
                &key_active,
                "sha256-WRONG",
                now
            ),
            VerificationResult::DigestMismatch
        ));

        // Success case
        assert!(matches!(
            verify_manifest("tenant_test", &manifest_ok, &key_active, digest_456, now),
            VerificationResult::Ok
        ));

        // Test SHA-384
        let digest_abc = "sha384-abc";
        let signature_abc_active =
            hex::encode(signing_key_active.sign(digest_abc.as_bytes()).to_bytes());
        let manifest_sha384 = Manifest {
            id: "pkg-c".to_string(),
            digest: digest_abc.to_string(),
            signature: signature_abc_active,
            kid: "active".to_string(),
        };

        // Verifying dash prefix support
        assert!(matches!(
            verify_manifest(
                "tenant_test",
                &manifest_sha384,
                &key_active,
                digest_abc,
                now
            ),
            VerificationResult::Ok
        ));

        // Test Key Mismatch
        let signing_key_wrong = SigningKey::generate(&mut csprng);
        let pub_key_wrong = hex::encode(signing_key_wrong.verifying_key().to_bytes());
        let key_wrong = TrustedKey {
            kid: "wrong".to_string(),
            alg: "Ed25519".to_string(),
            public_key: pub_key_wrong,
            status: KeyStatus::Active,
            retired_at: None,
            not_before: None,
            not_after: None,
        };
        assert!(matches!(
            verify_manifest("tenant_test", &manifest_ok, &key_wrong, "sha256-456", now),
            VerificationResult::KeyMismatch
        ));

        // Test Invalid Signature (Tampered Digest)
        // Manifest says digest="sha256-456". Signature is for "sha256-456".
        // If we modify manifest.signature manually to invalid:
        let mut manifest_tampered = manifest_ok.clone();
        manifest_tampered.signature = "deadbeef".to_string(); // Invalid hex signature or just garbage
        // Or valid hex but invalid signature
        assert!(matches!(
            verify_manifest(
                "tenant_test",
                &manifest_tampered,
                &key_active,
                "sha256-456",
                now
            ),
            VerificationResult::SignatureInvalid
        ));

        // Test Valid Signature but Wrong Key (Public key doesn't match private key used)
        let signing_key_other = SigningKey::generate(&mut csprng);
        let signature_other = hex::encode(signing_key_other.sign(digest_456.as_bytes()).to_bytes());
        let mut manifest_wrong_sig = manifest_ok.clone();
        manifest_wrong_sig.signature = signature_other;

        assert!(matches!(
            verify_manifest(
                "tenant_test",
                &manifest_wrong_sig,
                &key_active,
                "sha256-456",
                now
            ),
            VerificationResult::SignatureInvalid
        ));

        // Test Retired logic
        // 1. In Window -> OK
        assert!(matches!(
            verify_manifest(
                "tenant_test",
                &manifest_ok,
                &key_retired_ok,
                "sha256-456",
                now
            ),
            VerificationResult::Ok
        ));

        // 2. Out Window -> Fail
        assert!(matches!(
            verify_manifest(
                "tenant_test",
                &manifest_ok,
                &key_retired_fail,
                "sha256-456",
                now
            ),
            VerificationResult::KeyRetiredOutOfWindow
        ));

        // 2b. Missing retired_at must fail-closed
        assert!(matches!(
            verify_manifest(
                "tenant_test",
                &manifest_ok,
                &key_retired_none,
                "sha256-456",
                now
            ),
            VerificationResult::KeyRetiredOutOfWindow
        ));

        // 3. Next -> OK
        assert!(matches!(
            verify_manifest("tenant_test", &manifest_ok, &key_next, "sha256-456", now),
            VerificationResult::Ok
        ));

        // Unsupported algorithm should fail explicitly before signature verification.
        assert!(matches!(
            verify_manifest(
                "tenant_test",
                &manifest_ok,
                &key_wrong_alg,
                "sha256-456",
                now
            ),
            VerificationResult::SignatureAlgorithmMismatch
        ));

        // Test Key Not Yet Valid
        let mut key_future = key_active.clone(); // Removed duplicate definition
        key_future.not_before = Some(now + 100);
        assert!(matches!(
            verify_manifest("tenant_test", &manifest_ok, &key_future, "sha256-456", now),
            VerificationResult::KeyNotYetValid
        ));
        key_future.not_before = Some(now);
        assert!(matches!(
            verify_manifest("tenant_test", &manifest_ok, &key_future, "sha256-456", now),
            VerificationResult::Ok
        ));

        // Test Key Expired
        let mut key_expired = key_active.clone(); // Removed duplicate definition
        key_expired.not_after = Some(now - 100);
        assert!(matches!(
            verify_manifest("tenant_test", &manifest_ok, &key_expired, "sha256-456", now),
            VerificationResult::KeyExpired
        ));
        key_expired.not_after = Some(now);
        assert!(matches!(
            verify_manifest("tenant_test", &manifest_ok, &key_expired, "sha256-456", now),
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
