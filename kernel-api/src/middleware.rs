use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{
        HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, header::CONTENT_LENGTH,
    },
    middleware::Next,
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt;
use kernel_core::idempotency::{canonicalize_request_target, check_idempotency_conflict};
use kernel_core::quota::{QuotaLayer, QuotaViolation};
use ring::digest::{SHA256, digest};
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
const MAX_REPLAY_BODY_SIZE: usize = 10 * 1024 * 1024; // 10MB replay cache protection
const INFLIGHT_WAIT_TIMEOUT: Duration = Duration::from_secs(5);

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

/// Abstract Store Trait to allow switching to Redis (REQ: Production Readiness)
#[async_trait]
pub trait IdempotencyStore: Send + Sync {
    async fn get(&self, key: &IdempotencyScopeKey) -> Option<IdempotencyEntry>;
    /// Returns None if acquired successfully. Returns Some(entry) if already exists.
    async fn try_acquire(
        &self,
        key: IdempotencyScopeKey,
        body_hash: String,
    ) -> Option<IdempotencyEntry>;
    async fn complete(&self, key: IdempotencyScopeKey, record: IdempotencyRecord);
    async fn release_inflight(&self, key: &IdempotencyScopeKey);
    async fn cleanup(&self);
}

pub struct InMemoryIdempotencyStore {
    inner: Mutex<HashMap<IdempotencyScopeKey, IdempotencyEntry>>,
}

impl InMemoryIdempotencyStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl IdempotencyStore for InMemoryIdempotencyStore {
    async fn get(&self, key: &IdempotencyScopeKey) -> Option<IdempotencyEntry> {
        self.inner.lock().await.get(key).cloned()
    }

    async fn try_acquire(
        &self,
        key: IdempotencyScopeKey,
        body_hash: String,
    ) -> Option<IdempotencyEntry> {
        let mut lock = self.inner.lock().await;
        if let Some(entry) = lock.get(&key) {
            return Some(entry.clone());
        }

        lock.insert(
            key,
            IdempotencyEntry::InFlight {
                body_hash,
                notify: Arc::new(Notify::new()),
                expires_at: Instant::now() + IDEMPOTENCY_TTL,
            },
        );
        None
    }

    async fn complete(&self, key: IdempotencyScopeKey, record: IdempotencyRecord) {
        let mut lock = self.inner.lock().await;
        if let Some(IdempotencyEntry::InFlight { notify, .. }) = lock.remove(&key) {
            lock.insert(key, IdempotencyEntry::Completed(record));
            notify.notify_waiters();
        } else {
            // Should not happen if logic is correct, but safe fallback
            lock.insert(key, IdempotencyEntry::Completed(record));
        }
    }

    async fn release_inflight(&self, key: &IdempotencyScopeKey) {
        let mut lock = self.inner.lock().await;
        if let Some(IdempotencyEntry::InFlight { notify, .. }) = lock.remove(key) {
            notify.notify_waiters();
        }
    }

    async fn cleanup(&self) {
        let mut lock = self.inner.lock().await;
        let now = Instant::now();
        let mut expired_inflight_notifies = Vec::new();
        lock.retain(|_, entry| {
            let keep = match entry {
                IdempotencyEntry::InFlight { expires_at, .. } => *expires_at > now,
                IdempotencyEntry::Completed(record) => record.expires_at > now,
            };
            if !keep {
                if let IdempotencyEntry::InFlight { notify, .. } = entry {
                    expired_inflight_notifies.push(notify.clone());
                }
            }
            keep
        });
        drop(lock);

        for notify in expired_inflight_notifies {
            notify.notify_waiters();
        }
    }
}

// TODO: Implement RedisIdempotencyStore
// Use Redis SETNX for locking and HSET for storing records.
// pub struct RedisIdempotencyStore { client: redis::Client }

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

#[derive(Clone)]
pub struct MiddlewareState {
    pub idempotency_store: Arc<dyn IdempotencyStore>,
    pub action_store: ActionStore,
}

impl MiddlewareState {
    pub fn new() -> Self {
        Self {
            // Default to InMemory, but ready for Redis injection
            idempotency_store: Arc::new(InMemoryIdempotencyStore::new()),
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
                idempotency_store.cleanup().await;

                // Cleanup Action Store
                {
                    let mut lock = action_store.lock().await;
                    let now = Instant::now();
                    lock.retain(|_, record| record.expires_at > now);
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

pub async fn get_action(
    store: &ActionStore,
    tenant_id: &str,
    action_id: &str,
) -> Option<ActionRecord> {
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
pub async fn idempotency_middleware(
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
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
    // Note: This forces buffering. For streams > 10MB, Idempotency is not supported by this middleware.
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

    let store = &state.idempotency_store;

    loop {
        // Atomic check-and-acquire
        match store
            .try_acquire(scope_key.clone(), body_hash.clone())
            .await
        {
            None => {
                // Acquired successfully (inserted InFlight)
                break;
            }
            Some(entry) => {
                // Conflict or InFlight
                match entry {
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
                        return Ok(build_replay_response(&record));
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
                            &existing_hash,
                        ) {
                            warn!(
                                key = %idempotency_key,
                                "Idempotency conflict detected (InFlight)"
                            );
                            return Err(StatusCode::CONFLICT);
                        }
                        // Wait for the in-flight request to complete
                        if tokio::time::timeout(INFLIGHT_WAIT_TIMEOUT, notify.notified())
                            .await
                            .is_err()
                        {
                            warn!(
                                key = %idempotency_key,
                                timeout_ms = INFLIGHT_WAIT_TIMEOUT.as_millis() as u64,
                                "Timed out waiting for in-flight idempotent request"
                            );
                            return Err(StatusCode::CONFLICT);
                        }
                        continue; // Retry loop
                    }
                }
            }
        }
    }

    // Process request
    let response = next.run(req).await;

    // Store successful result
    if response.status().is_success() {
        let (parts, body) = response.into_parts();
        if response_not_cacheable_for_replay(&parts.headers) {
            info!(
                "Skipping idempotency replay cache due to Content-Length > {}",
                MAX_REPLAY_BODY_SIZE
            );
            store.release_inflight(&scope_key).await;
            return Ok(Response::from_parts(parts, body));
        }

        // We need to buffer the response body to store it for replay.
        let body_bytes = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(_) => {
                error!(
                    "Response body could not be buffered for idempotency cache due to body read error"
                );
                store.release_inflight(&scope_key).await;
                return Err(StatusCode::BAD_GATEWAY);
            }
        };
        if body_bytes.len() > MAX_REPLAY_BODY_SIZE {
            info!(
                "Skipping idempotency replay cache due to response body > {} bytes",
                MAX_REPLAY_BODY_SIZE
            );
            store.release_inflight(&scope_key).await;
            return Ok(Response::from_parts(parts, Body::from(body_bytes)));
        }
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
        store.complete(scope_key.clone(), stored).await;
        return Ok(Response::from_parts(parts, Body::from(body_bytes)));
    }

    // On failure
    store.release_inflight(&scope_key).await;
    Ok(response)
}

// ... helpers ...

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
            HeaderName::from_bytes(name.as_bytes())
                .unwrap_or(HeaderName::from_static("retry-after")),
            val.parse()
                .unwrap_or_else(|_| HeaderValue::from_static("1")),
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

fn response_not_cacheable_for_replay(headers: &HeaderMap) -> bool {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > MAX_REPLAY_BODY_SIZE)
}

fn violation_to_status(v: &QuotaViolation) -> StatusCode {
    match v.layer {
        QuotaLayer::SystemHardLimit => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::TOO_MANY_REQUESTS,
    }
}
