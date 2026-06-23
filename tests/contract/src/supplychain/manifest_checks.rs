#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};
    use kernel_core::supplychain::{
        verify_break_glass, verify_manifest, BreakGlassContext, KeyStatus, Manifest, TrustedKey,
        VerificationResult,
    };

    fn sign_digest(signing_key: &SigningKey, digest: &str) -> String {
        hex::encode(signing_key.sign(digest.as_bytes()).to_bytes())
    }

    fn active_key(signing_key: &SigningKey) -> TrustedKey {
        TrustedKey {
            kid: "active".to_string(),
            alg: "Ed25519".to_string(),
            status: KeyStatus::Active,
            retired_at: None,
            not_before: None,
            not_after: None,
            public_key: signing_key.verifying_key().to_bytes(),
        }
    }

    #[test]
    fn test_manifest_signature_trust_root() {
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);

        let digest_ok = "sha256-123";
        let manifest_ok = Manifest {
            id: "pkg-a".to_string(),
            digest: digest_ok.to_string(),
            signature: sign_digest(&signing_key, digest_ok),
            kid: "active".to_string(),
        };

        let now = 100_000;

        let key_active = active_key(&signing_key);

        assert!(matches!(
            verify_manifest(&manifest_ok, &key_active, digest_ok, now),
            VerificationResult::Ok
        ));

        let key_revoked = TrustedKey {
            status: KeyStatus::Revoked,
            ..key_active.clone()
        };
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_revoked, digest_ok, now),
            VerificationResult::KeyRevoked
        ));

        let key_wrong_kid = TrustedKey {
            kid: "other".to_string(),
            ..key_active.clone()
        };
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_wrong_kid, digest_ok, now),
            VerificationResult::KeyMismatch
        ));

        let mut manifest_bad_sig = manifest_ok.clone();
        manifest_bad_sig.signature = "invalid".to_string();
        assert!(matches!(
            verify_manifest(&manifest_bad_sig, &key_active, digest_ok, now),
            VerificationResult::SignatureInvalid
        ));

        let key_retired_in = TrustedKey {
            status: KeyStatus::Retired,
            retired_at: Some(now - 50),
            ..key_active.clone()
        };
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_retired_in, digest_ok, now),
            VerificationResult::Ok
        ));

        let key_retired_out = TrustedKey {
            status: KeyStatus::Retired,
            retired_at: Some(now - 90_000),
            ..key_active.clone()
        };
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_retired_out, digest_ok, now),
            VerificationResult::KeyRetiredOutOfWindow
        ));

        let key_next = TrustedKey {
            status: KeyStatus::Next,
            ..key_active.clone()
        };
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_next, digest_ok, now),
            VerificationResult::Ok
        ));

        let key_wrong_alg = TrustedKey {
            alg: "RS256".to_string(),
            ..key_active.clone()
        };
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_wrong_alg, digest_ok, now),
            VerificationResult::SignatureAlgorithmMismatch
        ));

        let key_nbf_future = TrustedKey {
            not_before: Some(now + 1_000),
            ..key_active.clone()
        };
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_nbf_future, digest_ok, now),
            VerificationResult::KeyNotYetValid
        ));

        let key_expired = TrustedKey {
            not_after: Some(now - 1_000),
            ..key_active
        };
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_expired, digest_ok, now),
            VerificationResult::KeyExpired
        ));
    }

    #[test]
    fn test_manifest_digest_format_and_match() {
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let key_active = active_key(&signing_key);

        let digest_ok = "sha384-abc";
        let manifest_ok = Manifest {
            id: "pkg-a".to_string(),
            digest: digest_ok.to_string(),
            signature: sign_digest(&signing_key, digest_ok),
            kid: "active".to_string(),
        };

        assert!(matches!(
            verify_manifest(&manifest_ok, &key_active, digest_ok, 100_000),
            VerificationResult::Ok
        ));

        assert!(matches!(
            verify_manifest(&manifest_ok, &key_active, "sha384-other", 100_000),
            VerificationResult::DigestMismatch
        ));

        let manifest_bad_prefix = Manifest {
            digest: "md5-abc".to_string(),
            signature: sign_digest(&signing_key, "md5-abc"),
            ..manifest_ok
        };

        assert!(matches!(
            verify_manifest(&manifest_bad_prefix, &key_active, "md5-abc", 100_000),
            VerificationResult::DigestMismatch
        ));
    }

    #[test]
    fn test_manifest_retired_acceptance_window() {
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let digest_ok = "sha256-123";
        let manifest_ok = Manifest {
            id: "pkg-a".to_string(),
            digest: digest_ok.to_string(),
            signature: sign_digest(&signing_key, digest_ok),
            kid: "active".to_string(),
        };
        let key_active = active_key(&signing_key);
        let now = 100_000;

        let key_retired_in = TrustedKey {
            status: KeyStatus::Retired,
            retired_at: Some(now - 50),
            ..key_active.clone()
        };
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_retired_in, digest_ok, now),
            VerificationResult::Ok
        ));

        let key_retired_out = TrustedKey {
            status: KeyStatus::Retired,
            retired_at: Some(now - 90_000),
            ..key_active
        };
        assert!(matches!(
            verify_manifest(&manifest_ok, &key_retired_out, digest_ok, now),
            VerificationResult::KeyRetiredOutOfWindow
        ));
    }

    #[test]
    fn test_manifest_break_glass_scope_and_ttl() {
        let ctx = BreakGlassContext {
            enabled: true,
            scope_tenant_id: Some("tenant_A".to_string()),
            scope_digest: Some("sha384-xyz".to_string()),
            expiry_ts: 100,
        };

        assert!(matches!(
            verify_break_glass(&ctx, "tenant_A", "sha384-xyz", 50),
            VerificationResult::Ok
        ));

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

        assert!(matches!(
            verify_break_glass(&ctx, "tenant_B", "sha384-xyz", 50),
            VerificationResult::BreakGlassScopeMismatch
        ));

        assert!(matches!(
            verify_break_glass(&ctx, "tenant_A", "sha384-xyz", 100),
            VerificationResult::BreakGlassExpired
        ));

        let ctx_global = BreakGlassContext {
            enabled: true,
            scope_tenant_id: None,
            scope_digest: Some("sha384-xyz".to_string()),
            expiry_ts: 200,
        };
        assert!(matches!(
            verify_break_glass(&ctx_global, "tenant_A", "sha384-xyz", 50),
            VerificationResult::BreakGlassScopeMissing
        ));
    }
}
