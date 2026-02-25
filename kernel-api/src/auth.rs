use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::DateTime;
use rusty_paseto::prelude::*;
use sea_orm::DatabaseConnection;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::middleware::BearerToken;
pub use kernel_core::auth::{TenantContext, TenantId, UserId};

#[derive(Debug)]
enum AuthError {
    Unauthorized,
    Forbidden,
}

#[derive(Deserialize)]
struct PasetoClaims {
    tenant_id: String,
    user_id: Option<String>,
    exp: String,
    nbf: Option<String>,
}

#[derive(Deserialize)]
struct PasetoFooter {
    kid: String,
}

/// REQ-AUTH-SOURCE: Extract TenantContext from token or dev-headers (if debug)
pub async fn auth_middleware(
    State(db): State<Arc<DatabaseConnection>>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    let (context, token_str) = if let Some(header) = req.headers().get("Authorization") {
        let value = header.to_str().map_err(|_| {
            tracing::warn!("Invalid Authorization header encoding");
            StatusCode::UNAUTHORIZED
        })?;
        let token_part = extract_bearer_token(value).ok_or_else(|| {
            tracing::warn!("Invalid Authorization header format");
            StatusCode::UNAUTHORIZED
        })?;
        match verify_paseto_v4_public_from_env(token_part) {
            Ok(ctx) => (ctx, token_part.to_string()),
            Err(AuthError::Unauthorized) => {
                tracing::warn!("PASETO token verification failed: Unauthorized");
                return Err(StatusCode::UNAUTHORIZED);
            }
            Err(AuthError::Forbidden) => {
                tracing::warn!("PASETO token verification failed: Forbidden (invalid claims)");
                return Err(StatusCode::FORBIDDEN);
            }
        }
    } else {
        #[cfg(feature = "test-utils")]
        {
            if let Some(tenant_id_header) = req.headers().get("X-Tenant-Id") {
                let tenant_id_str = tenant_id_header.to_str().map_err(|_| {
                    tracing::warn!("Invalid X-Tenant-Id header encoding");
                    StatusCode::FORBIDDEN
                })?;
                let tenant_id = TenantId::new(tenant_id_str).map_err(|_| {
                    tracing::warn!(tenant_id = %tenant_id_str, "Invalid tenant_id in X-Tenant-Id");
                    StatusCode::FORBIDDEN
                })?;

                let user_id = if let Some(user_id_header) = req.headers().get("X-User-Id") {
                    let id_str = user_id_header.to_str().map_err(|_| {
                        tracing::warn!("Invalid X-User-Id header encoding");
                        StatusCode::FORBIDDEN
                    })?;
                    Some(UserId::new(id_str).map_err(|_| {
                        tracing::warn!(user_id = %id_str, "Invalid user_id in X-User-Id");
                        StatusCode::FORBIDDEN
                    })?)
                } else {
                    None
                };

                (
                    TenantContext::new(tenant_id.clone(), user_id),
                    format!("dev-token:{}", tenant_id),
                )
            } else {
                tracing::warn!(
                    "Missing Authorization header (and no X-Tenant-Id for debug bypass)"
                );
                return Err(StatusCode::UNAUTHORIZED);
            }
        }

        #[cfg(not(feature = "test-utils"))]
        {
            tracing::warn!("Missing Authorization header");
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    req.extensions_mut().insert(context.with_db(db));
    req.extensions_mut().insert(BearerToken::new(token_str));
    Ok(next.run(req).await)
}

use std::sync::OnceLock;

static PASETO_PUBLIC_KEY: OnceLock<Vec<u8>> = OnceLock::new();
static PASETO_KEYSET: OnceLock<PasetoKeyset> = OnceLock::new();

pub fn is_auth_config_ready() -> bool {
    PASETO_PUBLIC_KEY.get().is_some() && PASETO_KEYSET.get().is_some()
}

#[derive(Debug)]
struct PasetoKeyset {
    active_kid: String,
    next_kids: HashSet<String>,
    retired_kids: HashSet<String>,
    revoked_kids: HashSet<String>,
    public_keys: HashMap<String, Vec<u8>>,
    allow_legacy_no_kid: bool,
}

impl PasetoKeyset {
    fn default_for_single_key() -> Self {
        Self {
            active_kid: "active".to_string(),
            next_kids: HashSet::new(),
            retired_kids: HashSet::new(),
            revoked_kids: HashSet::new(),
            public_keys: HashMap::new(),
            allow_legacy_no_kid: false,
        }
    }

    fn from_env() -> Result<Self, String> {
        let active_kid =
            std::env::var("FLEXI_PASETO_V4_ACTIVE_KID").unwrap_or_else(|_| "active".to_string());
        let next_kids = parse_kid_csv_env("FLEXI_PASETO_V4_NEXT_KIDS");
        let retired_kids = parse_kid_csv_env("FLEXI_PASETO_V4_RETIRED_KIDS");
        let revoked_kids = parse_kid_csv_env("FLEXI_PASETO_V4_REVOKED_KIDS");
        let allow_legacy_no_kid = parse_bool_env("FLEXI_PASETO_V4_ALLOW_LEGACY_NO_KID", false);
        let mut keyset = Self {
            active_kid,
            next_kids,
            retired_kids,
            revoked_kids,
            public_keys: HashMap::new(),
            allow_legacy_no_kid,
        };
        keyset.load_per_kid_public_keys_from_env()?;
        Ok(keyset)
    }

    fn load_per_kid_public_keys_from_env(&mut self) -> Result<(), String> {
        let mut load_key_for_kid = |kid: &str, required: bool| -> Result<(), String> {
            let env_key = kid_public_key_env_var_name(kid);
            match std::env::var(&env_key) {
                Ok(value) => {
                    let decoded = decode_public_key_b64url(&value, &env_key)?;
                    self.public_keys.insert(kid.to_string(), decoded);
                    Ok(())
                }
                Err(_) if required => Err(format!(
                    "{env_key} is required when kid '{kid}' is listed in FLEXI_PASETO_V4_NEXT_KIDS or FLEXI_PASETO_V4_RETIRED_KIDS"
                )),
                Err(_) => Ok(()),
            }
        };

        let require_explicit_active_key = self.active_kid != "active";
        load_key_for_kid(&self.active_kid, require_explicit_active_key)?;
        for kid in self.next_kids.iter().chain(self.retired_kids.iter()) {
            load_key_for_kid(kid, true)?;
        }
        Ok(())
    }

    fn public_key_for_kid(&self, kid: &str) -> Option<&[u8]> {
        self.public_keys.get(kid).map(Vec::as_slice)
    }

    fn with_default_public_key(mut self, key: Vec<u8>) -> Self {
        self.public_keys.insert(self.active_kid.clone(), key);
        self
    }

    fn validate_key_material(&self) -> Result<(), String> {
        let mut normalized_owner: HashMap<String, &str> = HashMap::new();
        for kid in std::iter::once(self.active_kid.as_str())
            .chain(self.next_kids.iter().map(String::as_str))
            .chain(self.retired_kids.iter().map(String::as_str))
        {
            let normalized = normalize_kid_for_env(kid);
            if let Some(existing) = normalized_owner.insert(normalized.clone(), kid) {
                if existing != kid {
                    return Err(format!(
                        "kid normalization collision: '{existing}' and '{kid}' map to FLEXI_PASETO_V4_PUBLIC_KEY_B64URL_{normalized}"
                    ));
                }
            }
        }

        if self.active_kid.trim().is_empty() {
            return Err("FLEXI_PASETO_V4_ACTIVE_KID must not be empty".to_string());
        }
        if self.revoked_kids.contains(&self.active_kid) {
            return Err(
                "active kid must not be listed in FLEXI_PASETO_V4_REVOKED_KIDS".to_string(),
            );
        }
        if self.next_kids.contains(&self.active_kid) || self.retired_kids.contains(&self.active_kid)
        {
            return Err(
                "active kid must not be included in FLEXI_PASETO_V4_NEXT_KIDS or FLEXI_PASETO_V4_RETIRED_KIDS"
                    .to_string(),
            );
        }
        if self
            .next_kids
            .iter()
            .any(|kid| self.retired_kids.contains(kid))
        {
            return Err(
                "same kid must not appear in both FLEXI_PASETO_V4_NEXT_KIDS and FLEXI_PASETO_V4_RETIRED_KIDS"
                    .to_string(),
            );
        }
        if self
            .revoked_kids
            .iter()
            .any(|kid| self.next_kids.contains(kid))
        {
            return Err(
                "same kid must not appear in both FLEXI_PASETO_V4_REVOKED_KIDS and FLEXI_PASETO_V4_NEXT_KIDS"
                    .to_string(),
            );
        }
        if self
            .revoked_kids
            .iter()
            .any(|kid| self.retired_kids.contains(kid))
        {
            return Err(
                "same kid must not appear in both FLEXI_PASETO_V4_REVOKED_KIDS and FLEXI_PASETO_V4_RETIRED_KIDS"
                    .to_string(),
            );
        }
        if self.public_keys.is_empty() && !self.is_legacy_without_kid_allowed() {
            return Err(
                "Auth keyset must include at least one public key (or enable legacy mode)"
                    .to_string(),
            );
        }
        for (kid, key) in &self.public_keys {
            if key.len() != 32 {
                return Err(format!(
                    "Public key for kid '{kid}' must be 32-byte Ed25519 public key"
                ));
            }
        }
        Ok(())
    }

    fn is_legacy_without_kid_allowed(&self) -> bool {
        self.allow_legacy_no_kid && self.revoked_kids.is_empty()
    }

    fn validate_token_kid(&self, kid: &str) -> Result<(), AuthError> {
        if self.revoked_kids.contains(kid) {
            return Err(AuthError::Unauthorized);
        }

        if kid == self.active_kid || self.next_kids.contains(kid) || self.retired_kids.contains(kid)
        {
            return Ok(());
        }

        // Return Unauthorized for unknown KIDs to avoid leaking keyset state
        Err(AuthError::Unauthorized)
    }
}

pub fn init_auth_config() -> Result<(), String> {
    let key_b64 = std::env::var("FLEXI_PASETO_V4_PUBLIC_KEY_B64URL")
        .map_err(|_| "FLEXI_PASETO_V4_PUBLIC_KEY_B64URL is not set".to_string())?;
    let decoded = decode_public_key_b64url(&key_b64, "FLEXI_PASETO_V4_PUBLIC_KEY_B64URL")?;
    let keyset = PasetoKeyset::from_env()?;
    init_auth_config_with_decoded_public_key_and_keyset(decoded, keyset)
}

pub fn init_auth_config_with_public_key_b64url(key: &str) -> Result<(), String> {
    init_auth_config_with_public_key_and_keyset(key, PasetoKeyset::default_for_single_key())
}

pub fn init_auth_config_with_public_key_and_revoked_kids(
    key: &str,
    revoked_kids: &[&str],
) -> Result<(), String> {
    init_auth_config_with_public_key_and_revoked_kids_and_legacy_mode(key, revoked_kids, false)
}

pub fn init_auth_config_with_public_key_and_revoked_kids_and_legacy_mode(
    key: &str,
    revoked_kids: &[&str],
    allow_legacy_no_kid: bool,
) -> Result<(), String> {
    let keyset = keyset_for_revoked_kids(revoked_kids, allow_legacy_no_kid);
    init_auth_config_with_public_key_and_keyset(key, keyset)
}

fn keyset_for_revoked_kids(revoked_kids: &[&str], allow_legacy_no_kid: bool) -> PasetoKeyset {
    let mut keyset = PasetoKeyset::default_for_single_key();
    keyset.revoked_kids = revoked_kids.iter().map(|v| (*v).to_string()).collect();
    keyset.allow_legacy_no_kid = allow_legacy_no_kid;
    keyset
}

fn init_auth_config_with_public_key_and_keyset(
    key: &str,
    keyset: PasetoKeyset,
) -> Result<(), String> {
    let decoded = decode_public_key_b64url(key, "FLEXI_PASETO_V4_PUBLIC_KEY_B64URL")?;
    let keyset = keyset.with_default_public_key(decoded.clone());
    init_auth_config_with_decoded_public_key_and_keyset(decoded, keyset)
}

fn init_auth_config_with_decoded_public_key_and_keyset(
    decoded: Vec<u8>,
    keyset: PasetoKeyset,
) -> Result<(), String> {
    keyset.validate_key_material()?;
    PASETO_PUBLIC_KEY
        .set(decoded)
        .map_err(|_| "Auth config already initialized".to_string())?;
    PASETO_KEYSET
        .set(keyset)
        .map_err(|_| "Auth keyset already initialized".to_string())
}

/// Verifies a PASETO v4.public token from an extracted bearer token.
fn verify_paseto_v4_public_from_env(token: &str) -> Result<TenantContext, AuthError> {
    let default_public_key = PASETO_PUBLIC_KEY.get().ok_or(AuthError::Unauthorized)?;
    let keyset = PASETO_KEYSET.get().ok_or(AuthError::Unauthorized)?;

    if has_legacy_paseto_layout(token) {
        if !keyset.is_legacy_without_kid_allowed() {
            if keyset.allow_legacy_no_kid && !keyset.revoked_kids.is_empty() {
                tracing::warn!("Legacy token mode is disabled while revoked kids are configured");
            }
            tracing::warn!("PASETO footer.kid is required but missing");
            return Err(AuthError::Unauthorized);
        }
        return verify_paseto_v4_public_token(token, default_public_key, None).map_err(|e| {
            tracing::warn!("PASETO signature or claim validation failed");
            e
        });
    }

    let (kid, footer_raw) = extract_footer_kid(token).map_err(|e| {
        tracing::warn!("PASETO footer missing or invalid");
        e
    })?;
    keyset.validate_token_kid(&kid).map_err(|e| {
        tracing::warn!(kid = %kid, "PASETO kid rejected");
        e
    })?;
    let key_for_kid = keyset.public_key_for_kid(&kid).ok_or_else(|| {
        tracing::warn!(kid = %kid, "No public key configured for kid");
        AuthError::Unauthorized
    })?;
    verify_paseto_v4_public_token(token, key_for_kid, Some(&footer_raw)).map_err(|e| {
        tracing::warn!("PASETO signature or claim validation failed");
        e
    })
}

/// Verifies a PASETO v4.public token using the provided public key.
fn verify_paseto_v4_public_token(
    token: &str,
    public_key_bytes: &[u8],
    footer_raw: Option<&str>,
) -> Result<TenantContext, AuthError> {
    let key_array: [u8; 32] = public_key_bytes
        .try_into()
        .map_err(|_| AuthError::Unauthorized)?;
    let key_raw = rusty_paseto::core::Key::<32>::from(key_array);
    let key = PasetoAsymmetricPublicKey::<V4, Public>::from(&key_raw);

    let mut parser = PasetoParser::<V4, Public>::default();
    if let Some(raw) = footer_raw {
        parser.set_footer(Footer::from(raw));
    }
    let verified_payload: serde_json::Value = parser
        .parse(token, &key)
        .map_err(|_| AuthError::Unauthorized)?;

    let claims: PasetoClaims =
        serde_json::from_value(verified_payload).map_err(|_| AuthError::Unauthorized)?;
    validate_claims(claims)
}

fn validate_claims(claims: PasetoClaims) -> Result<TenantContext, AuthError> {
    let tenant_id = TenantId::new(&claims.tenant_id).map_err(|_| AuthError::Forbidden)?;
    let user_id = if let Some(uid) = claims.user_id {
        Some(UserId::new(uid).map_err(|_| AuthError::Forbidden)?)
    } else {
        None
    };

    let now = unix_now();
    if let Some(nbf_str) = claims.nbf {
        let nbf_ts = DateTime::parse_from_rfc3339(&nbf_str)
            .map_err(|_| AuthError::Unauthorized)?
            .timestamp();
        if nbf_ts < 0 {
            return Err(AuthError::Unauthorized);
        }
        let nbf = nbf_ts as u64;
        if now < nbf {
            return Err(AuthError::Unauthorized);
        }
    }

    let exp_ts = DateTime::parse_from_rfc3339(&claims.exp)
        .map_err(|_| AuthError::Unauthorized)?
        .timestamp();
    if exp_ts < 0 {
        return Err(AuthError::Unauthorized);
    }
    let exp = exp_ts as u64;

    if now >= exp {
        return Err(AuthError::Unauthorized);
    }

    Ok(TenantContext::new(tenant_id, user_id))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_else(|e| {
            tracing::error!("SystemTime clock error: {e}");
            u64::MAX
        })
}

fn extract_bearer_token(auth_header: &str) -> Option<&str> {
    let (scheme, token) = auth_header.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Bearer") || token.is_empty() {
        return None;
    }
    Some(token)
}

fn parse_kid_csv_env(key: &str) -> HashSet<String> {
    std::env::var(key)
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_bool_env(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(raw) => {
            let trimmed = raw.trim().to_ascii_lowercase();
            match trimmed.as_str() {
                "1" | "true" | "yes" | "on" => true,
                "0" | "false" | "no" | "off" => false,
                _ => {
                    tracing::warn!(
                        key = %key,
                        value = %raw,
                        "Invalid boolean environment variable; using default"
                    );
                    default
                }
            }
        }
        Err(_) => default,
    }
}

fn decode_public_key_b64url(raw: &str, env_key: &str) -> Result<Vec<u8>, String> {
    let decoded = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| format!("{env_key} must be base64url (no padding) encoded"))?;
    if decoded.len() != 32 {
        return Err(format!(
            "{env_key} must decode to 32-byte Ed25519 public key"
        ));
    }
    Ok(decoded)
}

fn kid_public_key_env_var_name(kid: &str) -> String {
    let normalized = normalize_kid_for_env(kid);
    format!("FLEXI_PASETO_V4_PUBLIC_KEY_B64URL_{normalized}")
}

fn normalize_kid_for_env(kid: &str) -> String {
    kid.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
}

fn has_legacy_paseto_layout(token: &str) -> bool {
    token.split('.').count() == 3
}

/// Pre-parses the PASETO v4.public token to extract kid from the footer.
/// This manual pre-parse is required to reject revoked KIDs before running full signature verification.
/// CAUTION: This logic is coupled to the PASETO v4.public wire format (4 segments, base64url footer).
/// Update this if the PASETO spec or rusty_paseto serialization changes.
fn extract_footer_kid(token: &str) -> Result<(String, String), AuthError> {
    let parts = token.split('.').collect::<Vec<_>>();
    if parts.len() != 4 {
        return Err(AuthError::Unauthorized);
    }

    let version = parts[0];
    let purpose = parts[1];
    let footer_b64 = parts[3];

    if version != "v4" || purpose != "public" {
        return Err(AuthError::Unauthorized);
    }

    let footer_bytes = URL_SAFE_NO_PAD
        .decode(footer_b64)
        .map_err(|_| AuthError::Unauthorized)?;
    let footer_raw = String::from_utf8(footer_bytes).map_err(|_| AuthError::Unauthorized)?;
    let footer: PasetoFooter =
        serde_json::from_str(&footer_raw).map_err(|_| AuthError::Unauthorized)?;
    if footer.kid.trim().is_empty() {
        return Err(AuthError::Unauthorized);
    }
    Ok((footer.kid, footer_raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_core::auth::is_valid_principal;
    use std::sync::{Mutex, OnceLock as TestOnceLock};

    static ENV_TEST_LOCK: TestOnceLock<Mutex<()>> = TestOnceLock::new();

    fn with_env_test_lock<F: FnOnce()>(f: F) {
        let guard = ENV_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("ENV_TEST_LOCK poisoned");
        f();
        drop(guard);
    }

    #[test]
    fn extract_bearer_token_accepts_case_insensitive_scheme() {
        assert_eq!(extract_bearer_token("Bearer abc"), Some("abc"));
        assert_eq!(extract_bearer_token("bearer abc"), Some("abc"));
        assert_eq!(extract_bearer_token("BEARER abc"), Some("abc"));
    }

    #[test]
    fn extract_bearer_token_rejects_invalid_formats() {
        assert_eq!(extract_bearer_token("Token abc"), None);
        assert_eq!(extract_bearer_token("Bearer "), None);
        assert_eq!(extract_bearer_token("Bearer"), None);
    }

    #[test]
    fn is_valid_principal_allows_valid_chars() {
        assert!(is_valid_principal("tenant-1"));
        assert!(is_valid_principal("user.name"));
        assert!(is_valid_principal("system_admin"));
        assert!(is_valid_principal("1234567890"));
    }

    #[test]
    fn is_valid_principal_rejects_colon() {
        assert!(!is_valid_principal("tenant:1"));
        assert!(!is_valid_principal("urn:uuid:123"));
    }

    #[test]
    fn is_valid_principal_rejects_invalid_inputs() {
        assert!(!is_valid_principal("")); // Empty
        assert!(!is_valid_principal("a".repeat(129).as_str())); // Too long
        assert!(!is_valid_principal("tenant/1")); // Slash
        assert!(!is_valid_principal("tenant 1")); // Space
        assert!(!is_valid_principal("tenant@1")); // At
    }

    #[test]
    fn test_validate_token_kid() {
        let mut keyset = PasetoKeyset::default_for_single_key();
        keyset.next_kids.insert("next".to_string());
        keyset.retired_kids.insert("retired".to_string());
        keyset.revoked_kids.insert("revoked".to_string());

        assert!(keyset.validate_token_kid("active").is_ok());
        assert!(keyset.validate_token_kid("next").is_ok());
        assert!(keyset.validate_token_kid("retired").is_ok());
        assert!(matches!(
            keyset.validate_token_kid("revoked"),
            Err(AuthError::Unauthorized)
        ));
        assert!(matches!(
            keyset.validate_token_kid("unknown"),
            Err(AuthError::Unauthorized) // Should be Unauthorized, not Forbidden
        ));
    }

    #[test]
    fn test_extract_footer_kid_valid() {
        // v4.public.PAYLOAD.FOOTER(base64url of {"kid":"my-kid"})
        let footer_raw = r#"{"kid":"my-kid"}"#;
        let footer_b64 = URL_SAFE_NO_PAD.encode(footer_raw);
        let token = format!("v4.public.payload.{}", footer_b64);

        let (kid, footer) = extract_footer_kid(&token).unwrap();
        assert_eq!(kid, "my-kid");
        assert_eq!(footer, footer_raw);
    }

    #[test]
    fn test_extract_footer_kid_invalid() {
        assert!(extract_footer_kid("v3.public.p.f").is_err()); // Wrong version
        assert!(extract_footer_kid("v4.local.p.f").is_err()); // Wrong purpose
        assert!(extract_footer_kid("v4.public.p").is_err()); // Missing footer segment
        assert!(extract_footer_kid("v4.public.p.f.extra").is_err()); // Too many segments
        assert!(extract_footer_kid("v4.public.p.!!!").is_err()); // Invalid b64
        assert!(
            extract_footer_kid(&format!(
                "v4.public.p.{}",
                URL_SAFE_NO_PAD.encode(r#"{"not_kid":"val"}"#)
            ))
            .is_err()
        ); // Missing kid key
        assert!(
            extract_footer_kid(&format!(
                "v4.public.p.{}",
                URL_SAFE_NO_PAD.encode(r#"{"kid":" "}"#)
            ))
            .is_err()
        ); // Empty kid
    }

    #[test]
    fn test_parse_kid_csv_env() {
        with_env_test_lock(|| {
            temp_env::with_vars(
                [
                    ("KIDS_EMPTY", Some("")),
                    ("KIDS_SINGLE", Some("k1")),
                    ("KIDS_MULTI", Some(" k1, k2 ,k3,, ")),
                ],
                || {
                    assert!(parse_kid_csv_env("KIDS_EMPTY").is_empty());
                    let single = parse_kid_csv_env("KIDS_SINGLE");
                    assert_eq!(single.len(), 1);
                    assert!(single.contains("k1"));

                    let multi = parse_kid_csv_env("KIDS_MULTI");
                    assert_eq!(multi.len(), 3);
                    assert!(multi.contains("k1"));
                    assert!(multi.contains("k2"));
                    assert!(multi.contains("k3"));
                },
            );
        });
    }

    #[test]
    fn test_paseto_keyset_initialization() {
        // Successful default
        let default = PasetoKeyset::default_for_single_key();
        assert_eq!(default.active_kid, "active");
        assert!(!default.allow_legacy_no_kid);

        // From env
        let key_b64 = URL_SAFE_NO_PAD.encode([7_u8; 32]);
        let next_b64 = URL_SAFE_NO_PAD.encode([9_u8; 32]);
        with_env_test_lock(|| {
            temp_env::with_vars(
                [
                    ("FLEXI_PASETO_V4_ACTIVE_KID", Some("env-active")),
                    ("FLEXI_PASETO_V4_REVOKED_KIDS", Some("r1,r2")),
                    ("FLEXI_PASETO_V4_NEXT_KIDS", Some("next-a")),
                    (
                        "FLEXI_PASETO_V4_PUBLIC_KEY_B64URL_ENV_ACTIVE",
                        Some(&key_b64),
                    ),
                    ("FLEXI_PASETO_V4_PUBLIC_KEY_B64URL_NEXT_A", Some(&next_b64)),
                ],
                || {
                    let keyset = PasetoKeyset::from_env().unwrap();
                    assert_eq!(keyset.active_kid, "env-active");
                    assert!(keyset.revoked_kids.contains("r1"));
                    assert!(keyset.revoked_kids.contains("r2"));
                    assert!(!keyset.allow_legacy_no_kid);
                    assert_eq!(keyset.public_key_for_kid("env-active").unwrap(), [7_u8; 32]);
                    assert_eq!(keyset.public_key_for_kid("next-a").unwrap(), [9_u8; 32]);
                },
            );
        });
    }

    #[test]
    fn test_paseto_keyset_initialization_requires_kid_key_for_next() {
        with_env_test_lock(|| {
            temp_env::with_vars([("FLEXI_PASETO_V4_NEXT_KIDS", Some("next-a"))], || {
                let keyset = PasetoKeyset::from_env();
                assert!(keyset.is_err());
            });
        });
    }

    #[test]
    fn test_paseto_keyset_initialization_requires_explicit_active_key_when_customized() {
        with_env_test_lock(|| {
            temp_env::with_vars(
                [("FLEXI_PASETO_V4_ACTIVE_KID", Some("custom-active"))],
                || {
                    let keyset = PasetoKeyset::from_env();
                    assert!(keyset.is_err());
                },
            );
        });
    }

    #[test]
    fn test_paseto_keyset_validation_rejects_overlapping_kid_sets() {
        let mut keyset = PasetoKeyset::default_for_single_key();
        keyset
            .public_keys
            .insert(keyset.active_kid.clone(), vec![1_u8; 32]);
        keyset.revoked_kids.insert("active".to_string());
        assert!(keyset.validate_key_material().is_err());

        let mut keyset = PasetoKeyset::default_for_single_key();
        keyset
            .public_keys
            .insert(keyset.active_kid.clone(), vec![1_u8; 32]);
        keyset.next_kids.insert("k1".to_string());
        keyset.retired_kids.insert("k1".to_string());
        assert!(keyset.validate_key_material().is_err());
    }

    #[test]
    fn test_paseto_keyset_validation_rejects_kid_normalization_collision() {
        let mut keyset = PasetoKeyset::default_for_single_key();
        keyset.active_kid = "a-b".to_string();
        keyset.next_kids.insert("a_b".to_string());
        keyset.public_keys.insert("a-b".to_string(), vec![1_u8; 32]);
        keyset.public_keys.insert("a_b".to_string(), vec![2_u8; 32]);
        assert!(keyset.validate_key_material().is_err());
    }

    #[test]
    fn test_legacy_without_kid_blocked_when_revocation_exists() {
        let mut keyset = PasetoKeyset::default_for_single_key();
        keyset.revoked_kids.insert("revoked".to_string());
        assert!(!keyset.is_legacy_without_kid_allowed());
    }

    #[test]
    fn test_parse_bool_env() {
        with_env_test_lock(|| {
            temp_env::with_vars(
                [
                    ("BOOL_TRUE", Some("true")),
                    ("BOOL_FALSE", Some("0")),
                    ("BOOL_INVALID", Some("wat")),
                ],
                || {
                    assert!(parse_bool_env("BOOL_TRUE", false));
                    assert!(!parse_bool_env("BOOL_FALSE", true));
                    // Invalid input falls back to the provided default.
                    assert!(parse_bool_env("BOOL_INVALID", true));
                    assert!(!parse_bool_env("BOOL_MISSING", false));
                },
            );
        });
    }

    #[test]
    fn test_keyset_for_revoked_kids_disables_legacy_mode_by_default_api_contract() {
        let keyset = keyset_for_revoked_kids(&["revoked"], false);
        assert!(keyset.revoked_kids.contains("revoked"));
        assert!(!keyset.allow_legacy_no_kid);
        assert!(!keyset.is_legacy_without_kid_allowed());
    }

    #[test]
    fn test_paseto_keyset_no_fallback_to_generic() {
        let generic_key = [1_u8; 32];
        let kid_specific_key = [2_u8; 32];
        let kid_specific_b64 = URL_SAFE_NO_PAD.encode(kid_specific_key);

        with_env_test_lock(|| {
            // Case 1: active_kid is "active", but FLEXI_PASETO_V4_PUBLIC_KEY_B64URL_ACTIVE is missing.
            // Even if we provide the generic key to from_env, it should NOT be in public_keys["active"].
            temp_env::with_vars(
                [
                    ("FLEXI_PASETO_V4_ACTIVE_KID", Some("active")),
                    ("FLEXI_PASETO_V4_ALLOW_LEGACY_NO_KID", Some("true")),
                ],
                || {
                    let keyset = PasetoKeyset::from_env().unwrap();
                    assert!(keyset.public_key_for_kid("active").is_none());
                },
            );

            // Case 2: active_kid is "active", and FLEXI_PASETO_V4_PUBLIC_KEY_B64URL_ACTIVE IS present.
            // It should be used.
            temp_env::with_vars(
                [
                    ("FLEXI_PASETO_V4_ACTIVE_KID", Some("active")),
                    (
                        "FLEXI_PASETO_V4_PUBLIC_KEY_B64URL_ACTIVE",
                        Some(&kid_specific_b64),
                    ),
                ],
                || {
                    let keyset = PasetoKeyset::from_env().unwrap();
                    assert_eq!(
                        keyset.public_key_for_kid("active").unwrap(),
                        kid_specific_key
                    );
                },
            );
        });
    }
}
