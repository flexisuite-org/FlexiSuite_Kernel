use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::signature::{UnparsedPublicKey, ED25519};
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
    exp: Option<u64>,
    nbf: Option<u64>,
}

/// REQ-AUTH-SOURCE: Extract TenantContext from token or dev-headers (if debug)
pub async fn auth_middleware(mut req: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    let context = if let Some(header) = req.headers().get("Authorization") {
        let value = header.to_str().map_err(|_| StatusCode::UNAUTHORIZED)?;
        match verify_paseto_v4_public_from_env(value) {
            Ok(ctx) => ctx,
            Err(AuthError::Unauthorized) => return Err(StatusCode::UNAUTHORIZED),
            Err(AuthError::Forbidden) => return Err(StatusCode::FORBIDDEN),
        }
    } else {
        #[cfg(debug_assertions)]
        {
            if let Some(tenant_id) = req.headers().get("X-Tenant-Id") {
                let id = tenant_id.to_str().map_err(|_| StatusCode::FORBIDDEN)?;
                if !is_valid_principal(id) {
                    return Err(StatusCode::FORBIDDEN);
                }
                TenantContext {
                    tenant_id: id.to_string(),
                    user_id: None,
                }
            } else {
                return Err(StatusCode::UNAUTHORIZED);
            }
        }

        #[cfg(not(debug_assertions))]
        {
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    req.extensions_mut().insert(context);
    Ok(next.run(req).await)
}

fn verify_paseto_v4_public_from_env(auth_header: &str) -> Result<TenantContext, AuthError> {
    const PREFIX: &str = "Bearer ";
    if !auth_header.starts_with(PREFIX) {
        return Err(AuthError::Unauthorized);
    }

    let token = &auth_header[PREFIX.len()..];
    let public_key_b64 = std::env::var("FLEXI_PASETO_V4_PUBLIC_KEY_B64URL")
        .map_err(|_| AuthError::Unauthorized)?;
    let public_key = URL_SAFE_NO_PAD
        .decode(public_key_b64)
        .map_err(|_| AuthError::Unauthorized)?;
    if public_key.len() != 32 {
        return Err(AuthError::Unauthorized);
    }

    verify_paseto_v4_public_token(token, &public_key)
}

fn verify_paseto_v4_public_token(token: &str, public_key: &[u8]) -> Result<TenantContext, AuthError> {
    // Supports v4.public with optional footer (implicit assertion is not used in this API layer).
    let parts: Vec<&str> = token.split('.').collect();
    if !(parts.len() == 4 || parts.len() == 5) || parts[0] != "v4" || parts[1] != "public" {
        return Err(AuthError::Unauthorized);
    }

    let payload = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|_| AuthError::Unauthorized)?;
    let signature = URL_SAFE_NO_PAD
        .decode(parts[3])
        .map_err(|_| AuthError::Unauthorized)?;
    let footer = if parts.len() == 5 {
        URL_SAFE_NO_PAD
            .decode(parts[4])
            .map_err(|_| AuthError::Unauthorized)?
    } else {
        Vec::new()
    };

    let msg = pae(&[b"v4.public.", &payload, &footer, b""]);
    let verifier = UnparsedPublicKey::new(&ED25519, public_key);
    verifier
        .verify(&msg, &signature)
        .map_err(|_| AuthError::Unauthorized)?;

    let claims: PasetoClaims = serde_json::from_slice(&payload).map_err(|_| AuthError::Unauthorized)?;
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
    if let Some(nbf) = claims.nbf {
        if now < nbf {
            return Err(AuthError::Unauthorized);
        }
    }
    if let Some(exp) = claims.exp {
        if now >= exp {
            return Err(AuthError::Unauthorized);
        }
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
        && bytes
            .iter()
            .all(|b| matches!(*b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b':' | b'.'))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn pae(pieces: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(pieces.len() as u64).to_le_bytes());
    for piece in pieces {
        out.extend_from_slice(&(piece.len() as u64).to_le_bytes());
        out.extend_from_slice(piece);
    }
    out
}
