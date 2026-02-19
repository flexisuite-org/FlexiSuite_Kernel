use ring::signature;

#[derive(Debug, Clone)]
pub struct Manifest {
    pub id: String,
    pub digest: String,
    pub signature: String,
    pub kid: String,
}

#[derive(Debug, PartialEq, Eq, Clone)]
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
    KeyNotYetValid,
    KeyExpired,
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
    pub alg: String,
    pub public_key: String,
    pub status: KeyStatus,
    pub retired_at: Option<u64>,
    pub not_before: Option<u64>,
    pub not_after: Option<u64>,
}

/// Verify manifest signature and key status.
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
        return VerificationResult::DigestMismatch; // Malformed or unsupported digest
    }

    // 1b. Artifact Digest Verification (Contract: Manifest must match artifact)
    // Enforce mandatory check as per REQ-SUPPLYCHAIN-DIGEST-MATCH
    if manifest.digest != expected_artifact_digest {
        return VerificationResult::DigestMismatch;
    }

    // 2. Key ID Match (Contract: Key used must match Trusted Key)
    if manifest.kid != trusted_key.kid {
        // Better error classification for audit/triage
        return VerificationResult::KeyMismatch;
    }

    // 2b. Key Status Check
    match trusted_key.status {
        KeyStatus::Revoked => return VerificationResult::KeyRevoked,
        KeyStatus::Retired => {
            // Check Grace Window (e.g., 24h = 86400s)
            let grace_period = 86400;
            if let Some(retired_at) = trusted_key.retired_at {
                if now > retired_at.saturating_add(grace_period) {
                    return VerificationResult::KeyRetiredOutOfWindow;
                }
                // In window -> Proceed to signature check
            } else {
                // Retired but no timestamp -> Assume out
                return VerificationResult::KeyRetiredOutOfWindow;
            }
        }
        KeyStatus::Next => {
            // Verification allowed for Next keys (during rotation preparation)
        }
        KeyStatus::Active => {}
    }

    // 2c. Validity Period Check with Tolerance (30s)
    let tolerance = 30;
    if let Some(nbf) = trusted_key.not_before {
        if now < nbf.saturating_sub(tolerance) {
            return VerificationResult::KeyNotYetValid;
        }
    }
    if let Some(exp) = trusted_key.not_after {
        if now > exp.saturating_add(tolerance) {
            return VerificationResult::KeyExpired;
        }
    }

    // 3. Signature Verification (Real) via ring
    let signature_bytes = match hex::decode(&manifest.signature) {
        Ok(bytes) => bytes,
        Err(_) => return VerificationResult::SignatureInvalid,
    };

    let pub_key_bytes = match hex::decode(&trusted_key.public_key) {
        Ok(bytes) => bytes,
        Err(_) => return VerificationResult::SignatureInvalid,
    };

    let peer_public_key =
        signature::UnparsedPublicKey::new(&signature::ED25519, pub_key_bytes);

    // Verify signature over the UTF-8 bytes of the digest string
    match peer_public_key.verify(manifest.digest.as_bytes(), &signature_bytes) {
        Ok(()) => VerificationResult::Ok,
        Err(_) => VerificationResult::SignatureInvalid,
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
