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
use kernel_core::idempotency::canonicalize_request_target;
#[cfg(any(test, feature = "test-utils"))]
use kernel_core::quota::{QuotaLayer, QuotaViolation};
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify};
use tracing::{error, info, instrument, warn};

use crate::auth::TenantContext;

#[derive(Clone, Debug)]
pub struct MiddlewareConfig {
    pub idempotency_ttl: Duration,
    pub action_ttl: Duration,
    pub max_body_size: usize,
    pub max_replay_body_size: usize,
    pub inflight_wait_timeout: Duration,
}

impl Default for MiddlewareConfig {
    fn default() -> Self {
        fn get_env_duration(key: &str, default_secs: u64) -> Duration {
            match std::env::var(key) {
                Ok(v) => match v.parse::<u64>() {
                    Ok(s) => Duration::from_secs(s),
                    Err(_) => {
                        tracing::warn!(key = %key, value = %v, "Invalid Duration env var, using default");
                        Duration::from_secs(default_secs)
                    }
                },
                Err(_) => Duration::from_secs(default_secs),
            }
        }

        fn get_env_usize(key: &str, default_val: usize) -> usize {
            match std::env::var(key) {
                Ok(v) => match v.parse::<usize>() {
                    Ok(s) => s,
                    Err(_) => {
                        tracing::warn!(key = %key, value = %v, "Invalid usize env var, using default");
                        default_val
                    }
                },
                Err(_) => default_val,
            }
        }

        Self {
            idempotency_ttl: get_env_duration("IDEMPOTENCY_TTL_SECS", 24 * 60 * 60),
            action_ttl: get_env_duration("ACTION_TTL_SECS", 24 * 60 * 60),
            max_body_size: get_env_usize("MAX_BODY_SIZE_BYTES", 10 * 1024 * 1024),
            max_replay_body_size: get_env_usize("MAX_REPLAY_BODY_SIZE_BYTES", 10 * 1024 * 1024),
            inflight_wait_timeout: get_env_duration("INFLIGHT_WAIT_TIMEOUT_SECS", 5),
        }
    }
}

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
    tenant_id: kernel_core::auth::TenantId,
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
        ttl: Duration,
    ) -> Option<IdempotencyEntry>;
    async fn complete(&self, key: IdempotencyScopeKey, record: IdempotencyRecord);
    async fn release_inflight(&self, key: &IdempotencyScopeKey);
    async fn cleanup(&self);
}

pub struct InMemoryIdempotencyStore {
    inner: Mutex<HashMap<IdempotencyScopeKey, IdempotencyEntry>>,
}

impl Default for InMemoryIdempotencyStore {
    fn default() -> Self {
        Self::new()
    }
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
        ttl: Duration,
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
                expires_at: Instant::now() + ttl,
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
            if !keep && let IdempotencyEntry::InFlight { notify, .. } = entry {
                expired_inflight_notifies.push(notify.clone());
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
    tenant_id: kernel_core::auth::TenantId,
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
    pub config: MiddlewareConfig,
}

impl MiddlewareState {
    pub fn new(config: MiddlewareConfig) -> Self {
        Self::with_store(config, Arc::new(InMemoryIdempotencyStore::new()))
    }

    pub fn with_store(
        config: MiddlewareConfig,
        idempotency_store: Arc<dyn IdempotencyStore>,
    ) -> Self {
        Self {
            idempotency_store,
            action_store: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }

    /// Starts a background task to clean up expired idempotency and action records.
    ///
    /// # Warning
    /// This task runs indefinitely in a loop. The returned `JoinHandle` must be aborted
    /// to stop the task (e.g., during graceful shutdown).
    pub fn start_cleanup_task(&self) -> tokio::task::JoinHandle<()> {
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
        })
    }
}

pub async fn record_action(
    state: &MiddlewareState,
    tenant_id: kernel_core::auth::TenantId,
    action_id: &str,
    status: ActionStatus,
) {
    let mut lock = state.action_store.lock().await;
    // O(N) cleanup removed from critical path
    lock.insert(
        ActionScopeKey {
            tenant_id,
            action_id: action_id.to_string(),
        },
        ActionRecord {
            status,
            expires_at: Instant::now() + state.config.action_ttl,
        },
    );
}

pub async fn get_action(
    state: &MiddlewareState,
    tenant_id: kernel_core::auth::TenantId,
    action_id: &str,
) -> Option<ActionRecord> {
    let lock = state.action_store.lock().await;
    // O(N) cleanup removed from critical path
    lock.get(&ActionScopeKey {
        tenant_id,
        action_id: action_id.to_string(),
    })
    .filter(|r| r.expires_at > Instant::now())
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

    let idempotency_key = match parts.headers.get("Idempotency-Key") {
        Some(val) => {
            let key = match val.to_str() {
                Ok(k) => k,
                Err(_) => {
                    warn!("Invalid Idempotency-Key encoding");
                    return Err(StatusCode::BAD_REQUEST);
                }
            };
            if validate_idempotency_key(key).is_err() {
                warn!(key = %key, "Invalid Idempotency-Key format");
                return Err(StatusCode::BAD_REQUEST);
            }
            key.to_string()
        }
        None => {
            if method == Method::POST
                || method == Method::PUT
                || method == Method::DELETE
                || method == Method::PATCH
            {
                warn!(method = %method, "Missing Idempotency-Key for write operation");
                return Err(StatusCode::BAD_REQUEST);
            }
            return Ok(next.run(Request::from_parts(parts, body)).await);
        }
    };

    let tenant_ctx = parts
        .extensions
        .get::<TenantContext>()
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Enrich span
    tracing::Span::current().record("tenant_id", &tenant_ctx.tenant_id().to_string());
    tracing::Span::current().record("method", method.as_str());
    tracing::Span::current().record("path", parts.uri.path());

    // Canonicalize
    let path = parts.uri.path();
    let query = parts.uri.query();
    let canonical_target = canonicalize_request_target(path, query);

    // Uniqueness Scope: (tenant_id, method, canonical_target, key)
    let scope_key = IdempotencyScopeKey {
        tenant_id: tenant_ctx.tenant_id().clone(),
        method: method.as_str().to_string(),
        canonical_target: canonical_target.clone(),
        idempotency_key: idempotency_key.clone(),
    };

    // Body hash MUST be derived from the actual request body.
    // DoS Protection: Limit body size
    // Note: This forces buffering. For streams > 10MB, Idempotency is not supported by this middleware.

    // Check Store
    let state = parts
        .extensions
        .get::<MiddlewareState>()
        .cloned()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    let body_bytes = match to_bytes(body, state.config.max_body_size).await {
        Ok(b) => b,
        Err(_) => {
            warn!(
                "Request body exceeded max_body_size ({})",
                state.config.max_body_size
            );
            return Err(StatusCode::BAD_REQUEST);
        }
    };

    let body_hash = compute_body_hash(&body_bytes);
    let req = Request::from_parts(parts, Body::from(body_bytes));

    let store = &state.idempotency_store;
    let mut attempts = 0;
    const MAX_ATTEMPTS: usize = 3;

    loop {
        attempts += 1;
        if attempts > MAX_ATTEMPTS {
            warn!(
                key = %idempotency_key,
                attempts = attempts,
                "Exceeded max attempts waiting for in-flight idempotent request"
            );
            let mut res = StatusCode::SERVICE_UNAVAILABLE.into_response();
            let retry_after = state
                .config
                .inflight_wait_timeout
                .as_secs()
                .max(1)
                .to_string();
            if let Ok(val) = HeaderValue::from_str(&retry_after) {
                res.headers_mut()
                    .insert(axum::http::header::RETRY_AFTER, val);
            }
            return Ok(res);
        }

        // Atomic check-and-acquire
        match store
            .try_acquire(
                scope_key.clone(),
                body_hash.clone(),
                state.config.idempotency_ttl,
            )
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
                        // Scope key match implies canonical_target match.
                        // Conflict if body hash differs.
                        if body_hash != record.body_hash {
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
                        // Scope key match implies canonical_target match.
                        // Conflict if body hash differs.
                        if body_hash != existing_hash {
                            warn!(
                                key = %idempotency_key,
                                "Idempotency conflict detected (InFlight)"
                            );
                            return Err(StatusCode::CONFLICT);
                        }

                        // Wait for the in-flight request to complete
                        // Use enable() pattern to avoid missed wakeups if notify_waiters() fires
                        // between try_acquire returning and awaiting notified()
                        let notified = notify.notified();
                        tokio::pin!(notified);
                        notified.as_mut().enable();

                        if tokio::time::timeout(state.config.inflight_wait_timeout, notified)
                            .await
                            .is_err()
                        {
                            warn!(
                                key = %idempotency_key,
                                timeout_ms = state.config.inflight_wait_timeout.as_millis() as u64,
                                "Timed out waiting for in-flight idempotent request"
                            );
                            let mut res = StatusCode::SERVICE_UNAVAILABLE.into_response();
                            let retry_after = state
                                .config
                                .inflight_wait_timeout
                                .as_secs()
                                .max(1)
                                .to_string();
                            if let Ok(val) = HeaderValue::from_str(&retry_after) {
                                res.headers_mut()
                                    .insert(axum::http::header::RETRY_AFTER, val);
                            }
                            return Ok(res);
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
        if response_not_cacheable_for_replay(&parts.headers, state.config.max_replay_body_size) {
            info!(
                "Skipping idempotency replay cache due to Content-Length > {}",
                state.config.max_replay_body_size
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
                let mut res = Response::from_parts(parts, Body::empty());
                let val = HeaderValue::from_static("cache-buffer-error");
                res.headers_mut().insert("X-Idempotency-Cache-Error", val);
                return Ok(res);
            }
        };
        if body_bytes.len() > state.config.max_replay_body_size {
            info!(
                "Skipping idempotency replay cache due to response body > {} bytes",
                state.config.max_replay_body_size
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
            expires_at: Instant::now() + state.config.idempotency_ttl,
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
    use std::fmt::Write;
    let result = digest(&SHA256, body);
    let bytes = result.as_ref();
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        // Writing to a String cannot fail, but write! returns a Result.
        let _ = write!(hex, "{b:02x}");
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
            let name_str = name.as_str();
            // Filter transient headers
            if matches!(
                name_str,
                "date" | "content-length" | "transfer-encoding" | "connection" | "x-request-id"
            ) {
                return None;
            }
            value
                .to_str()
                .ok()
                .map(|v| (name_str.to_string(), v.to_string()))
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

#[cfg(any(test, feature = "test-utils"))]
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
    #[cfg(any(test, feature = "test-utils"))]
    {
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
    }

    Ok(next.run(req).await)
}

fn response_not_cacheable_for_replay(headers: &HeaderMap, max_size: usize) -> bool {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > max_size)
}

#[cfg(any(test, feature = "test-utils"))]
fn violation_to_status(v: &QuotaViolation) -> StatusCode {
    match v.layer {
        QuotaLayer::SystemHardLimit => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::TOO_MANY_REQUESTS,
    }
}
