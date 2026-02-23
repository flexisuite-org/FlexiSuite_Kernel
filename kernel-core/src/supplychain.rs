use ring::signature;
use tracing::warn;

pub const RETIRED_KEY_GRACE_PERIOD_SECS: u64 = 86_400;
pub const CLOCK_DRIFT_TOLERANCE_SECS: u64 = 30;

fn normalize_manifest_digest_for_compare(value: &str) -> Option<String> {
    let digest = value.trim();
    if let Some(hex_part) = digest.strip_prefix("sha256-") {
        if !hex_part.is_empty() && hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(format!("sha256-{}", hex_part.to_ascii_lowercase()));
        }
        return None;
    }
    if let Some(hex_part) = digest.strip_prefix("sha384-") {
        if !hex_part.is_empty() && hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(format!("sha384-{}", hex_part.to_ascii_lowercase()));
        }
        return None;
    }
    if !digest.is_empty() && digest.chars().all(|c| c.is_ascii_hexdigit()) {
        // Backward compatibility: legacy manifests may persist raw SHA-384 hex.
        return Some(format!("sha384-{}", digest.to_ascii_lowercase()));
    }
    None
}

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
    pub public_key: String,
    pub status: KeyStatus,
    pub retired_at: Option<u64>,
    pub not_before: Option<u64>,
    pub not_after: Option<u64>,
}

pub fn verify_manifest(
    tenant_id: &str,
    manifest: &Manifest,
    trusted_key: &TrustedKey,
    expected_artifact_digest: &str,
    now: u64,
) -> VerificationResult {
    let log_failure = |reason: &str| {
        warn!(
            event = "supplychain.verify_manifest.failed",
            tenant = %tenant_id,
            manifest_id = %manifest.id,
            kid = %trusted_key.kid,
            reason = reason,
            "Manifest verification failed"
        );
    };

    let normalized_manifest_digest = match normalize_manifest_digest_for_compare(&manifest.digest) {
        Some(v) => v,
        None => {
            log_failure("MANIFEST_DIGEST_FORMAT_INVALID");
            return VerificationResult::DigestMismatch;
        }
    };
    let normalized_expected_digest =
        match normalize_manifest_digest_for_compare(expected_artifact_digest) {
            Some(v) => v,
            None => {
                log_failure("MANIFEST_DIGEST_FORMAT_INVALID");
                return VerificationResult::DigestMismatch;
            }
        };
    if normalized_manifest_digest != normalized_expected_digest {
        log_failure("MANIFEST_DIGEST_MISMATCH");
        return VerificationResult::DigestMismatch;
    }

    if manifest.kid != trusted_key.kid {
        log_failure("MANIFEST_KEY_MISMATCH");
        return VerificationResult::KeyMismatch;
    }

    match trusted_key.status {
        KeyStatus::Revoked => {
            log_failure("MANIFEST_KEY_REVOKED");
            return VerificationResult::KeyRevoked;
        }
        KeyStatus::Retired => {
            if let Some(retired_at) = trusted_key.retired_at {
                if now > retired_at.saturating_add(RETIRED_KEY_GRACE_PERIOD_SECS) {
                    log_failure("MANIFEST_KEY_RETIRED_OUT_OF_WINDOW");
                    return VerificationResult::KeyRetiredOutOfWindow;
                }
            } else {
                log_failure("MANIFEST_KEY_RETIRED_OUT_OF_WINDOW");
                return VerificationResult::KeyRetiredOutOfWindow;
            }
        }
        KeyStatus::Next => {}
        KeyStatus::Active => {}
    }

    if let Some(nbf) = trusted_key.not_before {
        if now.saturating_add(CLOCK_DRIFT_TOLERANCE_SECS) < nbf {
            log_failure("MANIFEST_KEY_NOT_YET_VALID");
            return VerificationResult::KeyNotYetValid;
        }
    }
    if let Some(exp) = trusted_key.not_after {
        if now > exp.saturating_add(CLOCK_DRIFT_TOLERANCE_SECS) {
            log_failure("MANIFEST_KEY_EXPIRED");
            return VerificationResult::KeyExpired;
        }
    }

    if !trusted_key.alg.trim().eq_ignore_ascii_case("ed25519") {
        log_failure("MANIFEST_SIGNATURE_ALGORITHM_MISMATCH");
        return VerificationResult::SignatureAlgorithmMismatch;
    }

    let signature_bytes = match hex::decode(&manifest.signature) {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(
                event = "supplychain.verify_manifest.failed",
                tenant = %tenant_id,
                manifest_id = %manifest.id,
                kid = %trusted_key.kid,
                reason = "MANIFEST_SIGNATURE_INVALID",
                error_detail = %e,
                decode_stage = "signature_hex_decode",
                "Manifest verification failed"
            );
            return VerificationResult::SignatureInvalid;
        }
    };

    let pub_key_bytes = match hex::decode(&trusted_key.public_key) {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(
                event = "supplychain.verify_manifest.failed",
                tenant = %tenant_id,
                manifest_id = %manifest.id,
                kid = %trusted_key.kid,
                reason = "MANIFEST_SIGNATURE_INVALID",
                error_detail = %e,
                decode_stage = "public_key_hex_decode",
                "Manifest verification failed"
            );
            return VerificationResult::SignatureInvalid;
        }
    };

    let peer_public_key = signature::UnparsedPublicKey::new(&signature::ED25519, pub_key_bytes);
    match peer_public_key.verify(manifest.digest.as_bytes(), &signature_bytes) {
        Ok(()) => VerificationResult::Ok,
        Err(e) => {
            warn!(
                event = "supplychain.verify_manifest.failed",
                tenant = %tenant_id,
                manifest_id = %manifest.id,
                kid = %trusted_key.kid,
                reason = "MANIFEST_SIGNATURE_INVALID",
                error_detail = ?e,
                "Manifest verification failed"
            );
            VerificationResult::SignatureInvalid
        }
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
    if now >= ctx.expiry_ts {
        return VerificationResult::BreakGlassExpired;
    }

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
