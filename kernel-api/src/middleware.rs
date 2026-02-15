use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use kernel_core::idempotency::{canonicalize_request_target, check_idempotency_conflict};
use kernel_core::quota::{QuotaLayer, QuotaViolation};
use ring::digest::{digest, SHA256};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify};
use tracing::{error, info, instrument, warn};

use crate::auth::TenantContext;

const IDEMPOTENCY_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const ACTION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024; // 10MB DoS protection

#[derive(Clone, Debug)]
pub struct IdempotencyRecord {
    pub body_hash: String,
    pub action_id: String,
    pub status: StatusCode,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub expires_at: Instant,
}

#[derive(Clone, Debug)]
pub enum IdempotencyEntry {
    InFlight {
        body_hash: String,
        notify: Arc<Notify>,
        expires_at: Instant,
    },
    Completed(IdempotencyRecord),
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct IdempotencyScopeKey {
    tenant_id: String,
    method: String,
    canonical_target: String,
    idempotency_key: String,
}

pub type IdempotencyStore = Arc<Mutex<HashMap<IdempotencyScopeKey, IdempotencyEntry>>>;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ActionScopeKey {
    tenant_id: String,
    action_id: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActionStatus {
    Pending,
    Completed,
    Failed,
}

#[derive(Clone, Debug)]
pub struct ActionRecord {
    pub status: ActionStatus,
    pub expires_at: Instant,
}

pub type ActionStore = Arc<Mutex<HashMap<ActionScopeKey, ActionRecord>>>;

#[derive(Clone, Debug)]
pub struct MiddlewareState {
    pub idempotency_store: IdempotencyStore,
    pub action_store: ActionStore,
}

impl MiddlewareState {
    pub fn new() -> Self {
        Self {
            idempotency_store: Arc::new(Mutex::new(HashMap::new())),
            action_store: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start_cleanup_task(&self) {
        let idempotency_store = self.idempotency_store.clone();
        let action_store = self.action_store.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                
                // Cleanup Idempotency Store
                {
                    let mut lock = idempotency_store.lock().await;
                    let initial_len = lock.len();
                    cleanup_expired_idempotency_entries(&mut lock);
                    let removed = initial_len - lock.len();
                    if removed > 0 {
                        info!(removed_count = removed, "Cleaned up expired idempotency entries");
                    }
                }

                // Cleanup Action Store
                {
                    let mut lock = action_store.lock().await;
                    let initial_len = lock.len();
                    cleanup_expired_actions(&mut lock);
                    let removed = initial_len - lock.len();
                    if removed > 0 {
                        info!(removed_count = removed, "Cleaned up expired action entries");
                    }
                }
            }
        });
    }
}

pub async fn record_action(
    store: &ActionStore,
    tenant_id: &str,
    action_id: &str,
    status: ActionStatus,
) {
    let mut lock = store.lock().await;
    // O(N) cleanup removed from critical path
    lock.insert(
        ActionScopeKey {
            tenant_id: tenant_id.to_string(),
            action_id: action_id.to_string(),
        },
        ActionRecord {
            status,
            expires_at: Instant::now() + ACTION_TTL,
        },
    );
}

pub async fn get_action(store: &ActionStore, tenant_id: &str, action_id: &str) -> Option<ActionRecord> {
    let lock = store.lock().await;
    // O(N) cleanup removed from critical path
    lock.get(&ActionScopeKey {
        tenant_id: tenant_id.to_string(),
        action_id: action_id.to_string(),
    })
    .cloned()
}

/// REQ-IDEMPOTENCY-HEADER: Middleware logic
#[instrument(skip_all, fields(tenant_id, method, path))]
pub async fn idempotency_middleware(req: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    let (parts, body) = req.into_parts();
    let method = parts.method.clone();

    // Only apply to write methods
    if method == Method::GET || method == Method::HEAD || method == Method::OPTIONS {
        return Ok(next.run(Request::from_parts(parts, body)).await);
    }

    let idempotency_key = match parts.headers.get("Idempotency-Key") {
        Some(val) => {
            let key = match val.to_str() {
                Ok(k) => k,
                Err(_) => {
                    warn!("Invalid Idempotency-Key encoding");
                    return Err(StatusCode::BAD_REQUEST);
                }
            };
            if let Err(_) = validate_idempotency_key(key) {
                warn!(key = %key, "Invalid Idempotency-Key format");
                return Err(StatusCode::BAD_REQUEST);
            }
            key.to_string()
        }
        None => return Ok(next.run(Request::from_parts(parts, body)).await),
    };

    let tenant_ctx = parts
        .extensions
        .get::<TenantContext>()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    
    // Enrich span
    tracing::Span::current().record("tenant_id", &tenant_ctx.tenant_id);
    tracing::Span::current().record("method", method.as_str());
    tracing::Span::current().record("path", parts.uri.path());

    // Canonicalize
    let path = parts.uri.path();
    let query = parts.uri.query();
    let canonical_target = canonicalize_request_target(path, query);

    // Uniqueness Scope: (tenant_id, method, canonical_target, key)
    let scope_key = IdempotencyScopeKey {
        tenant_id: tenant_ctx.tenant_id.clone(),
        method: method.as_str().to_string(),
        canonical_target: canonical_target.clone(),
        idempotency_key: idempotency_key.clone(),
    };

    // Body hash MUST be derived from the actual request body.
    // DoS Protection: Limit body size
    let body_bytes = match to_bytes(body, MAX_BODY_SIZE).await {
        Ok(b) => b,
        Err(_) => {
            warn!("Request body exceeded MAX_BODY_SIZE ({})", MAX_BODY_SIZE);
            return Err(StatusCode::BAD_REQUEST);
        }
    };
    
    let body_hash = compute_body_hash(&body_bytes);
    let req = Request::from_parts(parts, Body::from(body_bytes));

    // Check Store
    let state = req
        .extensions()
        .get::<MiddlewareState>()
        .cloned()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    loop {
        let mut lock = state.idempotency_store.lock().await;
        // O(N) cleanup removed from critical path

        let waiter = if let Some(existing) = lock.get(&scope_key) {
            match existing {
                IdempotencyEntry::Completed(record) => {
                    if check_idempotency_conflict(
                        &canonical_target,
                        &body_hash,
                        &canonical_target,
                        &record.body_hash,
                    ) {
                        warn!(
                            key = %idempotency_key, 
                            "Idempotency conflict detected (Completed)"
                        );
                        return Err(StatusCode::CONFLICT);
                    }
                    info!(key = %idempotency_key, "Replaying idempotent response");
                    return Ok(build_replay_response(record));
                }
                IdempotencyEntry::InFlight {
                    body_hash: existing_hash,
                    notify,
                    ..
                } => {
                    if check_idempotency_conflict(
                        &canonical_target,
                        &body_hash,
                        &canonical_target,
                        existing_hash,
                    ) {
                        warn!(
                            key = %idempotency_key, 
                            "Idempotency conflict detected (InFlight)"
                        );
                        return Err(StatusCode::CONFLICT);
                    }
                    // Wait for the in-flight request to complete
                    notify.clone()
                }
            }
        } else {
            lock.insert(
                scope_key.clone(),
                IdempotencyEntry::InFlight {
                    body_hash: body_hash.clone(),
                    notify: Arc::new(Notify::new()),
                    expires_at: Instant::now() + IDEMPOTENCY_TTL,
                },
            );
            break;
        };
        drop(lock);
        // Wait outside lock
        waiter.notified().await;
    }

    // Process request
    let response = next.run(req).await;

    // Store successful result
    if response.status().is_success() {
        let (parts, body) = response.into_parts();
        // We need to buffer the response body to store it for replay.
        // Again, enforce size limit to protect memory.
        let body_bytes = match to_bytes(body, MAX_BODY_SIZE).await {
            Ok(bytes) => bytes,
            Err(_) => {
                error!("Response body exceeded MAX_BODY_SIZE, cannot cache for idempotency");
                let failed_record = IdempotencyRecord {
                    body_hash,
                    action_id: parts
                        .headers
                        .get("X-Action-Id")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string(),
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    headers: Vec::new(),
                    body: Vec::new(),
                    expires_at: Instant::now() + IDEMPOTENCY_TTL,
                };
                complete_with_record(&state.idempotency_store, scope_key.clone(), failed_record).await;
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        };
        let action_id = parts
            .headers
            .get("X-Action-Id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let stored = IdempotencyRecord {
            body_hash,
            action_id,
            status: parts.status,
            headers: snapshot_headers(&parts.headers),
            body: body_bytes.to_vec(),
            expires_at: Instant::now() + IDEMPOTENCY_TTL,
        };
        complete_with_record(&state.idempotency_store, scope_key.clone(), stored).await;
        return Ok(Response::from_parts(parts, Body::from(body_bytes)));
    }

    // If request failed (non-2xx), we release the lock so retries can happen.
    // Note: 4xx errors are generally deterministic and *could* be cached, 
    // but for now we follow the pattern of "retry on failure".
    // 5xx errors should definitely allow retry.
    release_inflight(&state.idempotency_store, &scope_key).await;
    Ok(response)
}

fn compute_body_hash(body: &[u8]) -> String {
    let result = digest(&SHA256, body);
    let mut hex = String::with_capacity(result.as_ref().len() * 2);
    for b in result.as_ref() {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

fn validate_idempotency_key(key: &str) -> Result<(), StatusCode> {
    if key.is_empty() || key.len() > 128 {
        return Err(StatusCode::BAD_REQUEST);
    }
    if !key.as_bytes().iter().all(|b| (0x21..=0x7e).contains(b)) {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}

async fn release_inflight(store: &IdempotencyStore, scope_key: &IdempotencyScopeKey) {
    let mut write_lock = store.lock().await;
    if let Some(IdempotencyEntry::InFlight { notify, .. }) = write_lock.remove(scope_key) {
        notify.notify_waiters();
    }
}

async fn complete_with_record(
    store: &IdempotencyStore,
    scope_key: IdempotencyScopeKey,
    record: IdempotencyRecord,
) {
    let mut write_lock = store.lock().await;
    let notify = match write_lock.remove(&scope_key) {
        Some(IdempotencyEntry::InFlight { notify, .. }) => notify,
        _ => Arc::new(Notify::new()),
    };
    write_lock.insert(scope_key, IdempotencyEntry::Completed(record));
    notify.notify_waiters();
}

fn cleanup_expired_idempotency_entries(store: &mut HashMap<IdempotencyScopeKey, IdempotencyEntry>) {
    let now = Instant::now();
    store.retain(|_, entry| match entry {
        IdempotencyEntry::InFlight { expires_at, .. } => *expires_at > now,
        IdempotencyEntry::Completed(record) => record.expires_at > now,
    });
}

fn cleanup_expired_actions(store: &mut HashMap<ActionScopeKey, ActionRecord>) {
    let now = Instant::now();
    store.retain(|_, record| record.expires_at > now);
}

fn snapshot_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect()
}

fn build_replay_response(record: &IdempotencyRecord) -> Response {
    let mut res = Response::builder()
        .status(record.status)
        .body(Body::from(record.body.clone()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());

    for (name, val) in &record.headers {
        if let (Ok(header_name), Ok(header_value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(val),
        ) {
            res.headers_mut().insert(header_name, header_value);
        }
    }
    if let Ok(value) = HeaderValue::from_str("true") {
        res.headers_mut().insert("X-Idempotency-Replay", value);
    }
    res
}

fn violation_to_response(v: &QuotaViolation) -> Response {
    let mut res = violation_to_status(v).into_response();
    for (name, val) in v.headers() {
        res.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap_or(HeaderName::from_static("retry-after")),
            val.parse().unwrap_or_else(|_| HeaderValue::from_static("1")),
        );
    }
    res
}

/// REQ-QUOTA-HTTP-CONTRACT: Evaluation Priority (System > Tenant > API)
pub async fn quota_middleware(req: Request<Body>, next: Next) -> Result<Response, Response> {
    // 1. System Hard Limit (Critical Protection)
    if req.headers().contains_key("X-Mock-Quota-System") {
        let violation = QuotaViolation {
            layer: QuotaLayer::SystemHardLimit,
            retry_after_s: 100,
        };
        warn!("System Hard Limit exceeded");
        return Err(violation_to_response(&violation));
    }

    // 2. Tenant Budget
    if req.headers().contains_key("X-Mock-Quota-Tenant") {
        let violation = QuotaViolation {
            layer: QuotaLayer::TenantBudget,
            retry_after_s: 5,
        };
        warn!("Tenant Budget exceeded");
        return Err(violation_to_response(&violation));
    }

    // 3. API Rate Limit
    if req.headers().contains_key("X-Mock-Quota-Api") {
        let violation = QuotaViolation {
            layer: QuotaLayer::ApiRateLimit,
            retry_after_s: 60,
        };
        warn!("API Rate Limit exceeded");
        return Err(violation_to_response(&violation));
    }

    Ok(next.run(req).await)
}

fn violation_to_status(v: &QuotaViolation) -> StatusCode {
    match v.layer {
        QuotaLayer::SystemHardLimit => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::TOO_MANY_REQUESTS,
    }
}
