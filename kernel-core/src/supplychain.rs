use ring::signature::{ED25519, UnparsedPublicKey};

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
    pub public_key: Vec<u8>,
    pub status: KeyStatus,
    pub retired_at: Option<u64>,
}

fn signature_scheme_for_digest(digest: &str) -> Option<&'static str> {
    if digest.starts_with("sha256-") {
        Some("ed25519-sha256")
    } else if digest.starts_with("sha384-") {
        Some("ed25519-sha384")
    } else {
        None
    }
}

fn manifest_signing_payload(manifest: &Manifest, scheme: &str) -> Vec<u8> {
    format!(
        "flexisuite-manifest:v1:{scheme}:{}:{}:{}",
        manifest.id, manifest.kid, manifest.digest
    )
    .into_bytes()
}

fn verify_signature(payload: &[u8], signature_hex: &str, public_key: &[u8]) -> bool {
    let signature = match hex::decode(signature_hex) {
        Ok(sig) => sig,
        Err(_) => return false,
    };
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(payload, &signature)
        .is_ok()
}

/// Mock verification with time-aware context
pub fn verify_manifest(
    manifest: &Manifest,
    trusted_key: &TrustedKey,
    expected_artifact_digest: &str,
    now: u64,
) -> VerificationResult {
    // 1. Digest Existence/Format Check + Scheme Selection
    // Spec: Must use "-" prefix (e.g., sha256-..., sha384-...)
    // REQ-SUPPLYCHAIN-DIGEST-FORMAT
    let scheme = match signature_scheme_for_digest(&manifest.digest) {
        Some(scheme) => scheme,
        None => {
            return VerificationResult::DigestMismatch; // Malformed or unsupported digest
        }
    };

    if trusted_key.public_key.is_empty() {
        return VerificationResult::SignatureInvalid;
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

    // 3. Cryptographic Signature Verification (fail closed)
    let payload = manifest_signing_payload(manifest, scheme);
    if !verify_signature(&payload, &manifest.signature, &trusted_key.public_key) {
        return VerificationResult::SignatureInvalid;
    }

    VerificationResult::Ok
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
