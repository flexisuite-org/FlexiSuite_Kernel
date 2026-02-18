use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::DateTime;
use rusty_paseto::prelude::*;
use serde::Deserialize;
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

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
pub async fn auth_middleware(mut req: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    let context = if let Some(header) = req.headers().get("Authorization") {
        let value = header.to_str().map_err(|_| {
            tracing::warn!("Invalid Authorization header encoding");
            StatusCode::UNAUTHORIZED
        })?;
        match verify_paseto_v4_public_from_env(value) {
            Ok(ctx) => ctx,
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
        #[cfg(debug_assertions)]
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

                TenantContext::new(tenant_id, user_id)
            } else {
                tracing::warn!(
                    "Missing Authorization header (and no X-Tenant-Id for debug bypass)"
                );
                return Err(StatusCode::UNAUTHORIZED);
            }
        }

        #[cfg(not(debug_assertions))]
        {
            tracing::warn!("Missing Authorization header");
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    req.extensions_mut().insert(context);
    Ok(next.run(req).await)
}

use std::sync::OnceLock;

static PASETO_PUBLIC_KEY: OnceLock<Vec<u8>> = OnceLock::new();
static PASETO_KEYSET: OnceLock<PasetoKeyset> = OnceLock::new();

#[derive(Debug)]
struct PasetoKeyset {
    active_kid: String,
    next_kids: HashSet<String>,
    retired_kids: HashSet<String>,
    revoked_kids: HashSet<String>,
}

impl PasetoKeyset {
    fn default_for_single_key() -> Self {
        Self {
            active_kid: "active".to_string(),
            next_kids: HashSet::new(),
            retired_kids: HashSet::new(),
            revoked_kids: HashSet::new(),
        }
    }

    fn from_env() -> Self {
        let active_kid =
            std::env::var("FLEXI_PASETO_V4_ACTIVE_KID").unwrap_or_else(|_| "active".to_string());
        let next_kids = parse_kid_csv_env("FLEXI_PASETO_V4_NEXT_KIDS");
        let retired_kids = parse_kid_csv_env("FLEXI_PASETO_V4_RETIRED_KIDS");
        let revoked_kids = parse_kid_csv_env("FLEXI_PASETO_V4_REVOKED_KIDS");
        Self {
            active_kid,
            next_kids,
            retired_kids,
            revoked_kids,
        }
    }

    fn validate_token_kid(&self, kid: &str) -> Result<(), AuthError> {
        if self.revoked_kids.contains(kid) {
            return Err(AuthError::Unauthorized);
        }

        if kid == self.active_kid || self.next_kids.contains(kid) || self.retired_kids.contains(kid)
        {
            return Ok(());
        }

        Err(AuthError::Forbidden)
    }
}

pub fn init_auth_config() -> Result<(), String> {
    let key = std::env::var("FLEXI_PASETO_V4_PUBLIC_KEY_B64URL")
        .map_err(|_| "FLEXI_PASETO_V4_PUBLIC_KEY_B64URL is not set".to_string())?;
    init_auth_config_with_public_key_and_keyset(&key, PasetoKeyset::from_env())
}

pub fn init_auth_config_with_public_key_b64url(key: &str) -> Result<(), String> {
    init_auth_config_with_public_key_and_keyset(key, PasetoKeyset::default_for_single_key())
}

pub fn init_auth_config_with_public_key_and_revoked_kids(
    key: &str,
    revoked_kids: &[&str],
) -> Result<(), String> {
    let mut keyset = PasetoKeyset::default_for_single_key();
    keyset.revoked_kids = revoked_kids.iter().map(|v| (*v).to_string()).collect();
    init_auth_config_with_public_key_and_keyset(key, keyset)
}

fn init_auth_config_with_public_key_and_keyset(
    key: &str,
    keyset: PasetoKeyset,
) -> Result<(), String> {
    let decoded = URL_SAFE_NO_PAD.decode(key).map_err(|_| {
        "FLEXI_PASETO_V4_PUBLIC_KEY_B64URL must be base64url (no padding)".to_string()
    })?;

    if decoded.len() != 32 {
        return Err(
            "FLEXI_PASETO_V4_PUBLIC_KEY_B64URL must decode to 32-byte Ed25519 public key"
                .to_string(),
        );
    }

    PASETO_PUBLIC_KEY
        .set(decoded)
        .map_err(|_| "Auth config already initialized".to_string())?;
    PASETO_KEYSET
        .set(keyset)
        .map_err(|_| "Auth keyset already initialized".to_string())
}

fn verify_paseto_v4_public_from_env(auth_header: &str) -> Result<TenantContext, AuthError> {
    let token = extract_bearer_token(auth_header).ok_or(AuthError::Unauthorized)?;
    let public_key = PASETO_PUBLIC_KEY.get().ok_or(AuthError::Unauthorized)?;
    let keyset = PASETO_KEYSET.get().ok_or(AuthError::Unauthorized)?;

    let (kid, footer_raw) = extract_footer_kid(token).map_err(|_| {
        tracing::warn!("PASETO footer missing or invalid");
        AuthError::Unauthorized
    })?;
    keyset.validate_token_kid(&kid).map_err(|e| {
        tracing::warn!(kid = %kid, "PASETO kid rejected");
        e
    })?;

    verify_paseto_v4_public_token(token, public_key, &footer_raw).map_err(|e| {
        tracing::warn!("PASETO signature or claim validation failed");
        e
    })
}

fn verify_paseto_v4_public_token(
    token: &str,
    public_key_bytes: &[u8],
    footer_raw: &str,
) -> Result<TenantContext, AuthError> {
    let key_array: [u8; 32] = public_key_bytes
        .try_into()
        .map_err(|_| AuthError::Unauthorized)?;
    let key_raw = rusty_paseto::core::Key::<32>::from(key_array);
    let key = PasetoAsymmetricPublicKey::<V4, Public>::from(&key_raw);

    let mut parser = PasetoParser::<V4, Public>::default();
    parser.set_footer(Footer::from(footer_raw));
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
}
