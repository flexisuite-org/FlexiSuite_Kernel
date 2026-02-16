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
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct TenantContext {
    pub tenant_id: String,
    pub user_id: Option<String>,
}

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
                let tenant_id = tenant_id_header.to_str().map_err(|_| {
                    tracing::warn!("Invalid X-Tenant-Id header encoding");
                    StatusCode::FORBIDDEN
                })?;
                if !is_valid_principal(tenant_id) {
                    tracing::warn!(tenant_id = %tenant_id, "Invalid tenant_id in X-Tenant-Id");
                    return Err(StatusCode::FORBIDDEN);
                }

                let user_id = if let Some(user_id_header) = req.headers().get("X-User-Id") {
                    let id = user_id_header.to_str().map_err(|_| {
                        tracing::warn!("Invalid X-User-Id header encoding");
                        StatusCode::FORBIDDEN
                    })?;
                    if !is_valid_principal(id) {
                        tracing::warn!(user_id = %id, "Invalid user_id in X-User-Id");
                        return Err(StatusCode::FORBIDDEN);
                    }
                    Some(id.to_string())
                } else {
                    None
                };

                TenantContext {
                    tenant_id: tenant_id.to_string(),
                    user_id,
                }
            } else {
                tracing::warn!("Missing Authorization header (and no X-Tenant-Id for debug bypass)");
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

pub fn init_auth_config() -> Result<(), String> {
    let key = std::env::var("FLEXI_PASETO_V4_PUBLIC_KEY_B64URL")
        .map_err(|_| "FLEXI_PASETO_V4_PUBLIC_KEY_B64URL is not set".to_string())?;
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
        .map_err(|_| "Auth config already initialized".to_string())
}

fn verify_paseto_v4_public_from_env(auth_header: &str) -> Result<TenantContext, AuthError> {
    let token = extract_bearer_token(auth_header).ok_or(AuthError::Unauthorized)?;
    let public_key = PASETO_PUBLIC_KEY.get().ok_or(AuthError::Unauthorized)?;

    verify_paseto_v4_public_token(token, public_key)
}

fn verify_paseto_v4_public_token(
    token: &str,
    public_key_bytes: &[u8],
) -> Result<TenantContext, AuthError> {
    let key_array: [u8; 32] = public_key_bytes
        .try_into()
        .map_err(|_| AuthError::Unauthorized)?;
    let key_raw = rusty_paseto::core::Key::<32>::from(key_array);
    let key = PasetoAsymmetricPublicKey::<V4, Public>::from(&key_raw);

    let verified_payload: serde_json::Value = PasetoParser::<V4, Public>::default()
        .parse(token, &key)
        .map_err(|_| AuthError::Unauthorized)?;

    let claims: PasetoClaims =
        serde_json::from_value(verified_payload).map_err(|_| AuthError::Unauthorized)?;
    validate_claims(claims)
}

fn validate_claims(claims: PasetoClaims) -> Result<TenantContext, AuthError> {
    if !is_valid_principal(&claims.tenant_id) {
        return Err(AuthError::Forbidden);
    }
    if let Some(user_id) = &claims.user_id {
        if !is_valid_principal(user_id) {
            return Err(AuthError::Forbidden);
        }
    }

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

    Ok(TenantContext {
        tenant_id: claims.tenant_id,
        user_id: claims.user_id,
    })
}

fn is_valid_principal(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes.iter().all(
            |b| matches!(*b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.'),
        )
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

#[cfg(test)]
mod tests {
    use super::{extract_bearer_token, is_valid_principal};

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
