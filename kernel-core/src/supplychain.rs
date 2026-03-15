#[derive(Debug, Clone)]
pub struct Manifest {
    pub id: String,
    pub digest: String,
    pub signature: String,
    pub kid: String,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum KeyStatus {
    Active,
    Next,
    Retired,
    Revoked,
}

#[derive(Debug)]
pub enum VerificationResult {
    Ok,
    DigestMismatch,
    SignatureAlgorithmMismatch,
    SignatureInvalid,
    KeyRevoked,
    KeyRetiredOutOfWindow,
    KeyNotYetValid,
    KeyExpired,
    BreakGlassExpired,
    BreakGlassScopeMismatch,
    BreakGlassDisabled,
    BreakGlassScopeMissing,
    KeyMismatch,
}

pub struct BreakGlassContext {
    pub enabled: bool,
    pub scope_tenant_id: Option<String>,
    pub scope_digest: Option<String>,
    pub expiry_ts: u64,
}

#[derive(Debug, Clone)]
pub struct TrustedKey {
    pub kid: String,
    pub alg: String,
    pub status: KeyStatus,
    pub retired_at: Option<u64>,
    pub not_before: Option<u64>,
    pub not_after: Option<u64>,
    pub public_key: [u8; 32],
}

const RETIRED_KEY_GRACE_PERIOD_SECONDS: u64 = 86_400;
const CLOCK_DRIFT_TOLERANCE_SECONDS: u64 = 30;

pub fn verify_manifest(
    manifest: &Manifest,
    trusted_key: &TrustedKey,
    expected_artifact_digest: &str,
    now: u64,
) -> VerificationResult {
    // REQ-SUPPLYCHAIN-DIGEST-FORMAT
    let has_valid_prefix =
        manifest.digest.starts_with("sha256-") || manifest.digest.starts_with("sha384-");
    if !has_valid_prefix {
        metrics::counter!("verification_result", "result" => "DigestMismatch", "flow" => "manifest").increment(1);
        return VerificationResult::DigestMismatch;
    }

    // REQ-SUPPLYCHAIN-DIGEST-MATCH
    if manifest.digest != expected_artifact_digest {
        metrics::counter!("verification_result", "result" => "DigestMismatch", "flow" => "manifest").increment(1);
        return VerificationResult::DigestMismatch;
    }

    if manifest.kid != trusted_key.kid {
        metrics::counter!("verification_result", "result" => "KeyMismatch", "flow" => "manifest")
            .increment(1);
        return VerificationResult::KeyMismatch;
    }

    match trusted_key.status {
        KeyStatus::Revoked => {
            metrics::counter!("verification_result", "result" => "KeyRevoked", "flow" => "manifest").increment(1);
            return VerificationResult::KeyRevoked;
        }
        KeyStatus::Retired => {
            if let Some(retired_at) = trusted_key.retired_at {
                if now >= retired_at.saturating_add(RETIRED_KEY_GRACE_PERIOD_SECONDS) {
                    metrics::counter!("verification_result", "result" => "KeyRetiredOutOfWindow", "flow" => "manifest")
                        .increment(1);
                    return VerificationResult::KeyRetiredOutOfWindow;
                }
            } else {
                metrics::counter!("verification_result", "result" => "KeyRetiredOutOfWindow", "flow" => "manifest")
                    .increment(1);
                return VerificationResult::KeyRetiredOutOfWindow;
            }
        }
        KeyStatus::Next | KeyStatus::Active => {}
    }

    if let Some(nbf) = trusted_key.not_before
        && now.saturating_add(CLOCK_DRIFT_TOLERANCE_SECONDS) < nbf {
            metrics::counter!("verification_result", "result" => "KeyNotYetValid", "flow" => "manifest").increment(1);
            return VerificationResult::KeyNotYetValid;
        }

    if let Some(exp) = trusted_key.not_after
        && now > exp.saturating_add(CLOCK_DRIFT_TOLERANCE_SECONDS) {
            metrics::counter!("verification_result", "result" => "KeyExpired", "flow" => "manifest").increment(1);
            return VerificationResult::KeyExpired;
        }

    if !trusted_key.alg.trim().eq_ignore_ascii_case("ed25519") {
        metrics::counter!("verification_result", "result" => "SignatureAlgorithmMismatch", "flow" => "manifest")
            .increment(1);
        return VerificationResult::SignatureAlgorithmMismatch;
    }

    #[cfg(feature = "test-utils")]
    {
        let bypass_allowed =
            cfg!(debug_assertions) || std::env::var_os("ENABLE_TEST_UTILS").is_some();

        if !bypass_allowed {
            tracing::error!(
                "test-utils signature bypass blocked: enable debug assertions or set ENABLE_TEST_UTILS"
            );
            metrics::counter!("verification_result", "result" => "SignatureInvalid", "flow" => "manifest").increment(1);
            return VerificationResult::SignatureInvalid;
        }

        if manifest.signature == "invalid" {
            metrics::counter!("verification_result", "result" => "SignatureInvalid", "flow" => "manifest").increment(1);
            return VerificationResult::SignatureInvalid;
        }
        metrics::counter!("verification_result", "result" => "Ok", "flow" => "manifest")
            .increment(1);
        return VerificationResult::Ok;
    }

    #[cfg(not(feature = "test-utils"))]
    {
        use ring::signature;

        let mut sig_bytes = [0u8; 64];
        if hex::decode_to_slice(&manifest.signature, &mut sig_bytes).is_err() {
            metrics::counter!("verification_result", "result" => "SignatureInvalid", "flow" => "manifest").increment(1);
            return VerificationResult::SignatureInvalid;
        }

        let peer_public_key =
            signature::UnparsedPublicKey::new(&signature::ED25519, &trusted_key.public_key);

        if peer_public_key
            .verify(manifest.digest.as_bytes(), &sig_bytes)
            .is_err()
        {
            metrics::counter!("verification_result", "result" => "SignatureInvalid", "flow" => "manifest").increment(1);
            return VerificationResult::SignatureInvalid;
        }

        metrics::counter!("verification_result", "result" => "Ok", "flow" => "manifest")
            .increment(1);
        VerificationResult::Ok
    }
}

pub fn verify_break_glass(
    ctx: &BreakGlassContext,
    tenant_id: &str,
    digest: &str,
    now: u64,
) -> VerificationResult {
    if !ctx.enabled {
        metrics::counter!("verification_result", "result" => "BreakGlassDisabled", "flow" => "break_glass").increment(1);
        return VerificationResult::BreakGlassDisabled;
    }

    if now >= ctx.expiry_ts {
        metrics::counter!("verification_result", "result" => "BreakGlassExpired", "flow" => "break_glass").increment(1);
        return VerificationResult::BreakGlassExpired;
    }

    match &ctx.scope_tenant_id {
        Some(scope_tid) => {
            if scope_tid != tenant_id {
                metrics::counter!("verification_result", "result" => "BreakGlassScopeMismatch", "flow" => "break_glass")
                    .increment(1);
                return VerificationResult::BreakGlassScopeMismatch;
            }
        }
        None => {
            metrics::counter!("verification_result", "result" => "BreakGlassScopeMissing", "flow" => "break_glass")
                .increment(1);
            return VerificationResult::BreakGlassScopeMissing;
        }
    }

    match &ctx.scope_digest {
        Some(scope_dig) => {
            if scope_dig != digest {
                metrics::counter!("verification_result", "result" => "BreakGlassScopeMismatch", "flow" => "break_glass")
                    .increment(1);
                return VerificationResult::BreakGlassScopeMismatch;
            }
        }
        None => {
            metrics::counter!("verification_result", "result" => "BreakGlassScopeMissing", "flow" => "break_glass")
                .increment(1);
            return VerificationResult::BreakGlassScopeMissing;
        }
    }

    metrics::counter!("verification_result", "result" => "Ok", "flow" => "break_glass")
        .increment(1);
    VerificationResult::Ok
}
