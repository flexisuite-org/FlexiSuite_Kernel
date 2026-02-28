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
    SignatureInvalid,
    KeyRevoked,
    KeyRetiredOutOfWindow,
    BreakGlassExpired,
    BreakGlassScopeMismatch,
    BreakGlassDisabled,
    BreakGlassScopeMissing,
    KeyMismatch, // New: explicit triage
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
    pub status: KeyStatus,
    pub retired_at: Option<u64>,
    pub public_key: [u8; 32],
}

const RETIRED_KEY_GRACE_PERIOD_SECONDS: u64 = 86400;

/// Verifies a manifest against a trusted key.
///
/// In `test-utils` builds, this performs a mock verification (time-aware but signature bypass).
/// In release/non-test builds, this performs real Ed25519 signature verification using `ring`.
pub fn verify_manifest(
    manifest: &Manifest,
    trusted_key: &TrustedKey,
    expected_artifact_digest: &str,
    now: u64,
) -> VerificationResult {
    // 1. Digest Existence/Format Check
    // Spec: Must use "-" prefix (e.g., sha256-..., sha384-...)
    // REQ-SUPPLYCHAIN-DIGEST-FORMAT
    let has_valid_prefix =
        manifest.digest.starts_with("sha256-") || manifest.digest.starts_with("sha384-");

    if !has_valid_prefix {
        metrics::counter!("verification_result", "result" => "DigestMismatch").increment(1);
        return VerificationResult::DigestMismatch; // Malformed or unsupported digest
    }

    // 1b. Artifact Digest Verification (Contract: Manifest must match artifact)
    // Enforce mandatory check as per REQ-SUPPLYCHAIN-DIGEST-MATCH
    if manifest.digest != expected_artifact_digest {
        metrics::counter!("verification_result", "result" => "DigestMismatch").increment(1);
        return VerificationResult::DigestMismatch;
    }

    // 2. Key ID Match (Contract: Key used must match Trusted Key)
    if manifest.kid != trusted_key.kid {
        // Better error classification for audit/triage
        metrics::counter!("verification_result", "result" => "KeyMismatch").increment(1);
        return VerificationResult::KeyMismatch;
    }

    // 2b. Key Status Check
    match trusted_key.status {
        KeyStatus::Revoked => {
            metrics::counter!("verification_result", "result" => "KeyRevoked").increment(1);
            return VerificationResult::KeyRevoked
        },
        KeyStatus::Retired => {
            // Check Grace Window
            if let Some(retired_at) = trusted_key.retired_at {
                if now >= retired_at.saturating_add(RETIRED_KEY_GRACE_PERIOD_SECONDS) {
                    metrics::counter!("verification_result", "result" => "KeyRetiredOutOfWindow").increment(1);
                    return VerificationResult::KeyRetiredOutOfWindow;
                }
                // In window -> Proceed to signature check
                metrics::counter!("verification_flow", "state" => "KeyRetiredInWindow").increment(1);
            } else {
                // Retired but no timestamp -> Assume out
                metrics::counter!("verification_flow", "state" => "KeyRetiredNoTimestamp").increment(1);
                metrics::counter!("verification_result", "result" => "KeyRetiredOutOfWindow").increment(1);
                return VerificationResult::KeyRetiredOutOfWindow;
            }
        }
        KeyStatus::Next => {
            // Verification allowed for Next keys (during rotation preparation)
        }
        KeyStatus::Active => {}
    }

    // 3. Signature Verification
    #[cfg(feature = "test-utils")]
    {
        if manifest.signature == "invalid" {
            metrics::counter!("verification_result", "result" => "SignatureInvalid").increment(1);
            return VerificationResult::SignatureInvalid;
        }
        metrics::counter!("verification_result", "result" => "Ok").increment(1);
        VerificationResult::Ok
    }

    #[cfg(not(feature = "test-utils"))]
    {
        use ring::signature;

        let mut sig_bytes = [0u8; 64];
        if hex::decode_to_slice(&manifest.signature, &mut sig_bytes).is_err() {
             metrics::counter!("verification_result", "result" => "SignatureInvalid").increment(1);
             return VerificationResult::SignatureInvalid;
        }

        let peer_public_key = signature::UnparsedPublicKey::new(
            &signature::ED25519,
            &trusted_key.public_key,
        );

        if peer_public_key.verify(manifest.digest.as_bytes(), &sig_bytes).is_err() {
             metrics::counter!("verification_result", "result" => "SignatureInvalid").increment(1);
             return VerificationResult::SignatureInvalid;
        }

        metrics::counter!("verification_result", "result" => "Ok").increment(1);
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
        metrics::counter!("verification_result", "result" => "BreakGlassDisabled").increment(1);
        return VerificationResult::BreakGlassDisabled;
    }
    // Strict Expiry: now >= expiry means expired
    if now >= ctx.expiry_ts {
        metrics::counter!("verification_result", "result" => "BreakGlassExpired").increment(1);
        return VerificationResult::BreakGlassExpired;
    }

    // Strict Scope: Global bypass is FORBIDDEN. Scopes must be present.
    match &ctx.scope_tenant_id {
        Some(scope_tid) => {
            if scope_tid != tenant_id {
                metrics::counter!("verification_result", "result" => "BreakGlassScopeMismatch").increment(1);
                return VerificationResult::BreakGlassScopeMismatch;
            }
        }
        None => {
            metrics::counter!("verification_result", "result" => "BreakGlassScopeMissing").increment(1);
            return VerificationResult::BreakGlassScopeMissing;
        }
    }

    match &ctx.scope_digest {
        Some(scope_dig) => {
            if scope_dig != digest {
                metrics::counter!("verification_result", "result" => "BreakGlassScopeMismatch").increment(1);
                return VerificationResult::BreakGlassScopeMismatch;
            }
        }
        None => {
            metrics::counter!("verification_result", "result" => "BreakGlassScopeMissing").increment(1);
            return VerificationResult::BreakGlassScopeMissing;
        }
    }

    metrics::counter!("verification_result", "result" => "Ok").increment(1);
    VerificationResult::Ok
}
