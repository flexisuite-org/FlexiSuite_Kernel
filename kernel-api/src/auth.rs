#![allow(clippy::manual_inspect)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::manual_map)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::implicit_saturating_sub)]
#![allow(clippy::needless_borrows_for_generic_args)]
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

#[derive(Debug, thiserror::Error)]
enum AuthError {
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Forbidden")]
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

/// REQ-AUTH-SOURCE: Extract TenantContext from token or dev-headers (test-only build path)
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
        let token_part = if let Some(token) = extract_bearer_token(value) {
            token.to_string()
        } else {
            return Err(StatusCode::UNAUTHORIZED);
        };

        match verify_paseto_v4_public_from_env_token(&token_part) {
            Ok(ctx) => (ctx, token_part),
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
        #[cfg(any(test, feature = "test-utils"))]
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
                        tracing::warn!("Invalid user_id in X-User-Id (format invalid)");
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

        #[cfg(not(any(test, feature = "test-utils")))]
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

/// Dynamic revocation overlay.
///
/// Kids added here are treated as revoked in addition to those in `PASETO_KEYSET.revoked_kids`.
/// Updated at runtime by [`start_kid_revocation_listener`] without restarting the process.
static REVOKED_KIDS_OVERRIDE: OnceLock<std::sync::RwLock<HashSet<String>>> = OnceLock::new();

fn revoked_kids_override() -> &'static std::sync::RwLock<HashSet<String>> {
    REVOKED_KIDS_OVERRIDE.get_or_init(|| std::sync::RwLock::new(HashSet::new()))
}

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
}

impl PasetoKeyset {
    fn default_for_single_key() -> Self {
        Self {
            active_kid: "active".to_string(),
            next_kids: HashSet::new(),
            retired_kids: HashSet::new(),
            revoked_kids: HashSet::new(),
            public_keys: HashMap::new(),
        }
    }

    fn from_env(default_public_key: Option<&[u8]>) -> Result<Self, String> {
        let active_kid =
            std::env::var("FLEXI_PASETO_V4_ACTIVE_KID").unwrap_or_else(|_| "active".to_string());
        let next_kids = parse_kid_csv_env("FLEXI_PASETO_V4_NEXT_KIDS");
        let retired_kids = parse_kid_csv_env("FLEXI_PASETO_V4_RETIRED_KIDS");
        let revoked_kids = parse_kid_csv_env("FLEXI_PASETO_V4_REVOKED_KIDS");
        let mut keyset = Self {
            active_kid,
            next_kids,
            retired_kids,
            revoked_kids,
            public_keys: HashMap::new(),
        };

        // Only insert generic key if active_kid is default ("active") AND key is provided
        if let Some(key) = default_public_key {
            if keyset.active_kid == "active" {
                keyset
                    .public_keys
                    .insert(keyset.active_kid.clone(), key.to_vec());
            }
        }

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

        // If active_kid != "active", it is REQUIRED to be loaded from explicit env var.
        let require_explicit_active_key = self.active_kid != "active";
        load_key_for_kid(&self.active_kid, require_explicit_active_key)?;

        // Next/Retired always required
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
        if self.public_keys.is_empty() {
            return Err("Auth keyset must include at least one public key".to_string());
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

const DEFAULT_ACTIVE_KID: &str = "active";

pub fn init_auth_config() -> Result<(), String> {
    // Attempt to load generic key, but only require it if it's the *only* source for the active key
    let default_key = match std::env::var("FLEXI_PASETO_V4_PUBLIC_KEY_B64URL") {
        Ok(key_b64) => Some(decode_public_key_b64url(
            &key_b64,
            "FLEXI_PASETO_V4_PUBLIC_KEY_B64URL",
        )?),
        Err(_) => None,
    };

    let keyset = PasetoKeyset::from_env(default_key.as_deref())?;

    // Check if we actually have the active key
    let active_key_bytes = keyset.public_key_for_kid(&keyset.active_kid)
        .ok_or_else(|| {
            if keyset.active_kid == DEFAULT_ACTIVE_KID {
                format!("Active key for kid '{}' failed to load. Set FLEXI_PASETO_V4_PUBLIC_KEY_B64URL or FLEXI_PASETO_V4_PUBLIC_KEY_B64URL_ACTIVE", keyset.active_kid)
            } else {
                format!("Active key for kid '{}' failed to load", keyset.active_kid)
            }
        })?
        .to_vec();

    init_auth_config_with_decoded_public_key_and_keyset(active_key_bytes, keyset)
}

pub fn init_auth_config_with_public_key_b64url(key: &str) -> Result<(), String> {
    init_auth_config_with_public_key_and_keyset(key, PasetoKeyset::default_for_single_key())
}

pub fn init_auth_config_with_public_key_and_revoked_kids(
    key: &str,
    revoked_kids: &[&str],
) -> Result<(), String> {
    let keyset = keyset_for_revoked_kids(revoked_kids);
    init_auth_config_with_public_key_and_keyset(key, keyset)
}
fn keyset_for_revoked_kids(revoked_kids: &[&str]) -> PasetoKeyset {
    let mut keyset = PasetoKeyset::default_for_single_key();
    keyset.revoked_kids = revoked_kids.iter().map(|v| (*v).to_string()).collect();
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

fn verify_paseto_v4_public_from_env_token(token: &str) -> Result<TenantContext, AuthError> {
    let keyset = PASETO_KEYSET.get().ok_or(AuthError::Unauthorized)?;

    let (kid, footer_raw) = extract_footer_kid(token).map_err(|e| {
        tracing::warn!("PASETO footer missing or invalid");
        e
    })?;
    keyset.validate_token_kid(&kid).map_err(|e| {
        tracing::warn!(kid = %kid, "PASETO kid rejected");
        e
    })?;
    // Dynamic revocation overlay: check kids revoked at runtime via Redis pub/sub.
    // This check runs after the static keyset check so statically-configured revocations
    // remain unaffected by the override state.
    if revoked_kids_override()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&kid)
    {
        tracing::warn!(kid = %kid, "PASETO kid revoked via dynamic override");
        return Err(AuthError::Unauthorized);
    }
    let key_for_kid = keyset.public_key_for_kid(&kid).ok_or_else(|| {
        tracing::warn!(kid = %kid, "No public key configured for kid");
        AuthError::Unauthorized
    })?;
    verify_paseto_v4_public_token(token, key_for_kid, Some(&footer_raw)).map_err(|e| {
        tracing::warn!("PASETO signature or claim validation failed");
        e
    })
}

/// Starts a background task that maintains the dynamic KID revocation list.
///
/// # Mechanism
///
/// 1. **Redis Pub/Sub** (primary): Subscribes to `flexi:auth:kid_revoked` and adds any
///    received KID string to [`REVOKED_KIDS_OVERRIDE`] immediately.
/// 2. **Polling fallback** (secondary): Every 30 seconds re-reads the
///    `FLEXI_PASETO_V4_REVOKED_KIDS` environment variable. This ensures that nodes which
///    miss a pub/sub event (due to Redis connection interruptions) eventually converge.
///    Combined with the 30-second polling interval this gives a worst-case propagation
///    latency of ≤ 60 seconds across all nodes, satisfying the p95 SLO.
///
/// # Arguments
///
/// * `client` – A Redis client used exclusively for the pub/sub connection. A dedicated
///   connection is required because pub/sub puts the connection into subscriber mode.
///
/// # Shutdown
///
/// The returned [`tokio::task::JoinHandle`] can be aborted to stop the task.
pub fn start_kid_revocation_listener(client: redis::Client) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Seed the override set with the static environment-variable list so that any
        // kids already in FLEXI_PASETO_V4_REVOKED_KIDS are immediately effective.
        {
            let env_kids = parse_kid_csv_env("FLEXI_PASETO_V4_REVOKED_KIDS");
            let mut guard = revoked_kids_override()
                .write()
                .unwrap_or_else(|e| e.into_inner());
            guard.extend(env_kids);
        }

        let mut poll_interval = tokio::time::interval(std::time::Duration::from_secs(30));

        // Outer reconnect loop for pub/sub.
        'reconnect: loop {
            let mut pubsub = loop {
                match client.get_async_pubsub().await {
                    Ok(p) => break p,
                    Err(e) => {
                        tracing::warn!("KID revocation listener: pub/sub connect failed: {e}");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            };
            if let Err(e) = pubsub.subscribe("flexi:auth:kid_revoked").await {
                tracing::warn!("KID revocation listener: subscribe failed: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue 'reconnect;
            }
            tracing::info!("KID revocation listener: subscribed to flexi:auth:kid_revoked");

            use futures_util::StreamExt as _;
            let mut stream = pubsub.on_message();
            loop {
                tokio::select! {
                    // Polling branch: refresh from env every 30 s.
                    _ = poll_interval.tick() => {
                        let fresh = parse_kid_csv_env("FLEXI_PASETO_V4_REVOKED_KIDS");
                        if !fresh.is_empty() {
                            let mut guard = revoked_kids_override()
                                .write()
                                .unwrap_or_else(|e| e.into_inner());
                            guard.extend(fresh);
                        }
                    }
                    // Pub/Sub branch: react to real-time revocation events.
                    msg_opt = stream.next() => {
                        match msg_opt {
                            Some(msg) => {
                                let kid: String = match msg.get_payload() {
                                    Ok(k) => k,
                                    Err(e) => {
                                        tracing::warn!("KID revocation listener: invalid payload: {e}");
                                        continue;
                                    }
                                };
                                if !kid.trim().is_empty() {
                                    tracing::info!(kid = %kid, "KID dynamically revoked via Redis pub/sub");
                                    let mut guard = revoked_kids_override()
                                        .write()
                                        .unwrap_or_else(|e| e.into_inner());
                                    guard.insert(kid);
                                }
                            }
                            None => {
                                // Stream closed; break inner loop to reconnect.
                                tracing::warn!("KID revocation listener: pub/sub stream closed, reconnecting");
                                break;
                            }
                        }
                    }
                }
            }
        }
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
                    let default_key = [1_u8; 32];
                    let keyset = PasetoKeyset::from_env(Some(&default_key)).unwrap();
                    assert_eq!(keyset.active_kid, "env-active");
                    assert!(keyset.revoked_kids.contains("r1"));
                    assert!(keyset.revoked_kids.contains("r2"));
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
                let keyset = PasetoKeyset::from_env(Some(&[1_u8; 32]));
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
                    // Pass None for generic key to explicitly test lookup logic
                    let keyset = PasetoKeyset::from_env(None);
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
    fn test_keyset_for_revoked_kids_populates_revoked_kids() {
        let keyset = keyset_for_revoked_kids(&["revoked"]);
        assert!(keyset.revoked_kids.contains("revoked"));
    }
}
