#[derive(Debug, Clone)]
pub struct Manifest {
    pub id: String,
    pub digest: String,
    pub signature: String,
    pub kid: String,
}

#[derive(Debug, PartialEq, Eq)]
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

pub struct TrustedKey {
    pub kid: String,
    pub status: KeyStatus,
    pub retired_at: Option<u64>,
    pub public_key: Vec<u8>,
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
        // metrics::inc_verification_result("DigestMismatch");
        return VerificationResult::DigestMismatch; // Malformed or unsupported digest
    }

    // 1b. Artifact Digest Verification (Contract: Manifest must match artifact)
    // Enforce mandatory check as per REQ-SUPPLYCHAIN-DIGEST-MATCH
    if manifest.digest != expected_artifact_digest {
        // metrics::inc_verification_result("DigestMismatch");
        return VerificationResult::DigestMismatch;
    }

    // 2. Key ID Match (Contract: Key used must match Trusted Key)
    if manifest.kid != trusted_key.kid {
        // Better error classification for audit/triage
        // metrics::inc_verification_result("KeyMismatch");
        return VerificationResult::KeyMismatch;
    }

    // 2b. Key Status Check
    match trusted_key.status {
        KeyStatus::Revoked => {
            // metrics::inc_verification_result("KeyRevoked");
            return VerificationResult::KeyRevoked
        },
        KeyStatus::Retired => {
            // Check Grace Window
            if let Some(retired_at) = trusted_key.retired_at {
                if now >= retired_at.saturating_add(RETIRED_KEY_GRACE_PERIOD_SECONDS) {
                    // metrics::inc_verification_result("KeyRetiredOutOfWindow");
                    return VerificationResult::KeyRetiredOutOfWindow;
                }
                // In window -> Proceed to signature check
            } else {
                // Retired but no timestamp -> Assume out
                // metrics::inc_verification_result("KeyRetiredOutOfWindow");
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
            // metrics::inc_verification_result("SignatureInvalid");
            return VerificationResult::SignatureInvalid;
        }
        // metrics::inc_verification_result("Ok");
        VerificationResult::Ok
    }

    #[cfg(not(feature = "test-utils"))]
    {
        use ring::signature;
        let peer_public_key = signature::UnparsedPublicKey::new(
            &signature::ED25519,
            &trusted_key.public_key,
        );

        let sig_bytes = match hex::decode(&manifest.signature) {
             Ok(b) => b,
             Err(_) => {
                 // metrics::inc_verification_result("SignatureInvalid");
                 return VerificationResult::SignatureInvalid
             },
        };

        if peer_public_key.verify(manifest.digest.as_bytes(), &sig_bytes).is_err() {
             // metrics::inc_verification_result("SignatureInvalid");
             return VerificationResult::SignatureInvalid;
        }

        // metrics::inc_verification_result("Ok");
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
        return VerificationResult::BreakGlassDisabled;
    }
    // Strict Expiry: now >= expiry means expired
    if now >= ctx.expiry_ts {
        return VerificationResult::BreakGlassExpired;
    }

    // Strict Scope: Global bypass is FORBIDDEN. Scopes must be present.
    match &ctx.scope_tenant_id {
        Some(scope_tid) => {
            if scope_tid != tenant_id {
                return VerificationResult::BreakGlassScopeMismatch;
            }
        }
        None => return VerificationResult::BreakGlassScopeMissing,
    }

    match &ctx.scope_digest {
        Some(scope_dig) => {
            if scope_dig != digest {
                return VerificationResult::BreakGlassScopeMismatch;
            }
        }
        None => return VerificationResult::BreakGlassScopeMissing,
    }

    VerificationResult::Ok
}
