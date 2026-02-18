use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{
        HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, header::CONTENT_LENGTH,
    },
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::prelude::*;
use http_body_util::BodyExt;
use kernel_core::idempotency::canonicalize_request_target;
use kernel_core::quota::{QuotaLayer, QuotaViolation};
use redis::AsyncCommands;
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
    pub redis_url: String,
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

        fn get_env_string(key: &str, default_val: &str) -> String {
            std::env::var(key).unwrap_or_else(|_| default_val.to_string())
        }

        Self {
            idempotency_ttl: get_env_duration("IDEMPOTENCY_TTL_SECS", 24 * 60 * 60),
            action_ttl: get_env_duration("ACTION_TTL_SECS", 24 * 60 * 60),
            max_body_size: get_env_usize("MAX_BODY_SIZE_BYTES", 10 * 1024 * 1024),
            max_replay_body_size: get_env_usize("MAX_REPLAY_BODY_SIZE_BYTES", 10 * 1024 * 1024),
            inflight_wait_timeout: get_env_duration("INFLIGHT_WAIT_TIMEOUT_SECS", 5),
            redis_url: get_env_string("REDIS_URL", "redis://127.0.0.1:6379"),
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
    pub tenant_id: kernel_core::auth::TenantId,
    pub method: String,
    pub canonical_target: String,
    pub idempotency_key: String,
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

pub struct RedisIdempotencyStore {
    manager: redis::aio::ConnectionManager,
}

impl RedisIdempotencyStore {
    pub fn new(manager: redis::aio::ConnectionManager) -> Self {
        Self { manager }
    }

    fn format_key(key: &IdempotencyScopeKey) -> String {
        format!("idemp:{}:{}:{}:{}", key.tenant_id, key.method, key.canonical_target, key.idempotency_key)
    }
}

#[derive(Serialize, Deserialize)]
struct RedisIdempotencyRecordDto {
    body_hash: String,
    action_id: String,
    status: u16,
    headers: Vec<(String, String)>,
    body: String, // Base64 encoded
}

#[async_trait]
impl IdempotencyStore for RedisIdempotencyStore {
    async fn get(&self, key: &IdempotencyScopeKey) -> Option<IdempotencyEntry> {
        let mut conn = self.manager.clone();
        let redis_key = Self::format_key(key);
        let val: Option<String> = match conn.get(&redis_key).await {
            Ok(v) => v,
            Err(e) => {
                error!("Redis get error: {}", e);
                return None;
            }
        };

        match val {
            Some(s) if s.starts_with("IN_FLIGHT:") => {
                 let hash = s.trim_start_matches("IN_FLIGHT:").to_string();
                 let notify = Arc::new(Notify::new());
                 let n = notify.clone();
                 // Polling simulation
                 tokio::spawn(async move {
                     tokio::time::sleep(Duration::from_millis(200)).await;
                     n.notify_one();
                 });
                 Some(IdempotencyEntry::InFlight {
                     body_hash: hash,
                     notify,
                     expires_at: Instant::now() + Duration::from_secs(60),
                 })
            },
            Some(s) => {
                if let Ok(dto) = serde_json::from_str::<RedisIdempotencyRecordDto>(&s) {
                    let body_bytes = BASE64_STANDARD.decode(&dto.body).unwrap_or_default();
                    Some(IdempotencyEntry::Completed(IdempotencyRecord {
                        body_hash: dto.body_hash,
                        action_id: dto.action_id,
                        status: StatusCode::from_u16(dto.status).unwrap_or(StatusCode::OK),
                        headers: dto.headers,
                        body: body_bytes,
                        expires_at: Instant::now() + Duration::from_secs(3600),
                    }))
                } else {
                    None
                }
            },
            None => None,
        }
    }

    async fn try_acquire(
        &self,
        key: IdempotencyScopeKey,
        body_hash: String,
        ttl: Duration,
    ) -> Option<IdempotencyEntry> {
        let mut conn = self.manager.clone();
        let redis_key = Self::format_key(&key);
        let val = format!("IN_FLIGHT:{}", body_hash);
        let opts = redis::SetOptions::default().conditional_set(redis::ExistenceCheck::NX).with_expiration(redis::SetExpiry::EX(ttl.as_secs()));

        let res: Result<bool, _> = conn.set_options(&redis_key, val, opts).await;

        match res {
            Ok(true) => None, // Acquired
            Ok(false) => {
                self.get(&key).await
            },
            Err(e) => {
                error!("Redis set error: {}", e);
                None
            }
        }
    }

    async fn complete(&self, key: IdempotencyScopeKey, record: IdempotencyRecord) {
        let mut conn = self.manager.clone();
        let redis_key = Self::format_key(&key);
        let dto = RedisIdempotencyRecordDto {
            body_hash: record.body_hash,
            action_id: record.action_id,
            status: record.status.as_u16(),
            headers: record.headers,
            body: BASE64_STANDARD.encode(&record.body),
        };

        if let Ok(json) = serde_json::to_string(&dto) {
            let now = Instant::now();
            let ttl_secs = if record.expires_at > now {
                record.expires_at.duration_since(now).as_secs()
            } else {
                1
            };

            let _: Result<(), _> = conn.set_ex(&redis_key, json, ttl_secs).await;
        }
    }

    async fn release_inflight(&self, key: &IdempotencyScopeKey) {
        let mut conn = self.manager.clone();
        let redis_key = Self::format_key(key);
        let _: Result<(), _> = conn.del(&redis_key).await;
    }

    async fn cleanup(&self) {
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ActionScopeKey {
    pub tenant_id: kernel_core::auth::TenantId,
    pub action_id: String,
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

#[async_trait]
pub trait ActionStore: Send + Sync {
    async fn record(&self, tenant_id: kernel_core::auth::TenantId, action_id: &str, status: ActionStatus, ttl: Duration);
    async fn get(&self, tenant_id: kernel_core::auth::TenantId, action_id: &str) -> Option<ActionRecord>;
    async fn cleanup(&self);
}

pub struct InMemoryActionStore {
    inner: Mutex<HashMap<ActionScopeKey, ActionRecord>>,
}

impl Default for InMemoryActionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryActionStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ActionStore for InMemoryActionStore {
    async fn record(&self, tenant_id: kernel_core::auth::TenantId, action_id: &str, status: ActionStatus, ttl: Duration) {
        let mut lock = self.inner.lock().await;
        lock.insert(
            ActionScopeKey {
                tenant_id,
                action_id: action_id.to_string(),
            },
            ActionRecord {
                status,
                expires_at: Instant::now() + ttl,
            },
        );
    }

    async fn get(&self, tenant_id: kernel_core::auth::TenantId, action_id: &str) -> Option<ActionRecord> {
        let lock = self.inner.lock().await;
        lock.get(&ActionScopeKey {
            tenant_id,
            action_id: action_id.to_string(),
        })
        .filter(|r| r.expires_at > Instant::now())
        .cloned()
    }

    async fn cleanup(&self) {
        let mut lock = self.inner.lock().await;
        let now = Instant::now();
        lock.retain(|_, record| record.expires_at > now);
    }
}

pub struct RedisActionStore {
    manager: redis::aio::ConnectionManager,
}

impl RedisActionStore {
    pub fn new(manager: redis::aio::ConnectionManager) -> Self {
        Self { manager }
    }

    fn format_key(tenant_id: &str, action_id: &str) -> String {
        format!("action:{}:{}", tenant_id, action_id)
    }
}

#[derive(Serialize, Deserialize)]
struct RedisActionRecordDto {
    status: ActionStatus,
    expires_at_ts: u64,
}

#[async_trait]
impl ActionStore for RedisActionStore {
    async fn record(&self, tenant_id: kernel_core::auth::TenantId, action_id: &str, status: ActionStatus, ttl: Duration) {
        let mut conn = self.manager.clone();
        let key = Self::format_key(tenant_id.as_str(), action_id);

        let expires_at_ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + ttl.as_secs();

        let dto = RedisActionRecordDto {
            status,
            expires_at_ts,
        };

        if let Ok(json) = serde_json::to_string(&dto) {
            let _: Result<(), _> = conn.set_ex(&key, json, ttl.as_secs()).await;
        }
    }

    async fn get(&self, tenant_id: kernel_core::auth::TenantId, action_id: &str) -> Option<ActionRecord> {
        let mut conn = self.manager.clone();
        let key = Self::format_key(tenant_id.as_str(), action_id);
        let val: Option<String> = match conn.get(&key).await {
            Ok(v) => v,
            Err(_) => return None,
        };

        if let Some(s) = val {
            if let Ok(dto) = serde_json::from_str::<RedisActionRecordDto>(&s) {
                 let now_ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                 let remaining = if dto.expires_at_ts > now_ts {
                     dto.expires_at_ts - now_ts
                 } else {
                     0
                 };
                 Some(ActionRecord {
                     status: dto.status,
                     expires_at: Instant::now() + Duration::from_secs(remaining),
                 })
            } else {
                None
            }
        } else {
            None
        }
    }

    async fn cleanup(&self) {
    }
}

#[async_trait]
pub trait QuotaStore: Send + Sync {
    async fn check_and_update(&self, tenant_id: &kernel_core::auth::TenantId, layer: QuotaLayer) -> Result<(), QuotaViolation>;
}

pub struct InMemoryQuotaStore;

impl Default for InMemoryQuotaStore {
    fn default() -> Self {
        Self
    }
}

impl InMemoryQuotaStore {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl QuotaStore for InMemoryQuotaStore {
    async fn check_and_update(&self, _tenant_id: &kernel_core::auth::TenantId, _layer: QuotaLayer) -> Result<(), QuotaViolation> {
        Ok(())
    }
}

pub struct RedisQuotaStore {
    manager: redis::aio::ConnectionManager,
    script: redis::Script,
}

impl RedisQuotaStore {
    pub fn new(manager: redis::aio::ConnectionManager) -> Self {
        let script = redis::Script::new(r#"
            local key = KEYS[1]
            local rate = tonumber(ARGV[1])
            local capacity = tonumber(ARGV[2])
            local cost = tonumber(ARGV[3])
            local now = tonumber(ARGV[4])

            local tokens = tonumber(redis.call("HGET", key, "tokens"))
            local last_refill = tonumber(redis.call("HGET", key, "last_refill"))

            if tokens == nil then
                tokens = capacity
                last_refill = now
            end

            local delta = math.max(0, now - last_refill)
            local filled = math.min(capacity, tokens + (delta * rate))

            if filled >= cost then
                local new_tokens = filled - cost
                redis.call("HSET", key, "tokens", new_tokens, "last_refill", now)
                redis.call("PEXPIRE", key, 60000)
                return {1, new_tokens}
            else
                local required = cost - filled
                local retry_after = required / rate
                return {0, retry_after}
            end
        "#);
        Self { manager, script }
    }
}

#[async_trait]
impl QuotaStore for RedisQuotaStore {
    async fn check_and_update(&self, tenant_id: &kernel_core::auth::TenantId, layer: QuotaLayer) -> Result<(), QuotaViolation> {
        let (key, rate, capacity, cost) = match layer {
            QuotaLayer::SystemHardLimit => (
                "quota:system".to_string(),
                1000.0, // 1000 req/s
                1000.0,
                1.0,
            ),
            QuotaLayer::TenantBudget => (
                format!("quota:tenant:{}:cpu", tenant_id),
                1000.0, // 1000ms/s
                3000.0, // 3000ms burst
                5.0,    // 5ms cost estimate
            ),
            QuotaLayer::ApiRateLimit => (
                format!("quota:tenant:{}:api", tenant_id),
                16.666, // ~1000 req/min
                100.0,  // burst
                1.0,
            ),
            QuotaLayer::CircuitBreaker => {
                return Ok(());
            }
        };

        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();

        let mut conn = self.manager.clone();

        let res: Result<(i32, f64), _> = self.script.key(&key)
            .arg(rate).arg(capacity).arg(cost).arg(now)
            .invoke_async(&mut conn).await;

        match res {
            Ok((allowed, val)) => {
                if allowed == 1 {
                    Ok(())
                } else {
                    Err(QuotaViolation {
                        layer,
                        retry_after_s: val.ceil() as u64,
                    })
                }
            },
            Err(e) => {
                error!("Redis script error: {}", e);
                Ok(())
            }
        }
    }
}

#[derive(Clone)]
pub struct MiddlewareState {
    pub idempotency_store: Arc<dyn IdempotencyStore>,
    pub action_store: Arc<dyn ActionStore>,
    pub quota_store: Arc<dyn QuotaStore>,
    pub config: MiddlewareConfig,
}

impl MiddlewareState {
    pub async fn new(config: MiddlewareConfig) -> Self {
        Self::new_with_redis(config).await
    }

    pub async fn new_with_redis(config: MiddlewareConfig) -> Self {
        let client = redis::Client::open(config.redis_url.clone());
        match client {
            Ok(c) => {
                match c.get_connection_manager().await {
                    Ok(manager) => {
                         Self::with_store(
                            config,
                            Arc::new(RedisIdempotencyStore::new(manager.clone())),
                            Arc::new(RedisActionStore::new(manager.clone())),
                            Arc::new(RedisQuotaStore::new(manager.clone())),
                        )
                    },
                    Err(e) => {
                        error!("Failed to create Redis connection manager: {}", e);
                         Self::with_store(
                            config,
                            Arc::new(InMemoryIdempotencyStore::new()),
                            Arc::new(InMemoryActionStore::new()),
                            Arc::new(InMemoryQuotaStore::new()),
                        )
                    }
                }
            },
            Err(e) => {
                error!("Invalid Redis URL: {}", e);
                Self::with_store(
                    config,
                    Arc::new(InMemoryIdempotencyStore::new()),
                    Arc::new(InMemoryActionStore::new()),
                    Arc::new(InMemoryQuotaStore::new()),
                )
            }
        }
    }

    pub fn with_store(
        config: MiddlewareConfig,
        idempotency_store: Arc<dyn IdempotencyStore>,
        action_store: Arc<dyn ActionStore>,
        quota_store: Arc<dyn QuotaStore>,
    ) -> Self {
        Self {
            idempotency_store,
            action_store,
            quota_store,
            config,
        }
    }

    pub fn start_cleanup_task(&self) -> tokio::task::JoinHandle<()> {
        let idempotency_store = self.idempotency_store.clone();
        let action_store = self.action_store.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                idempotency_store.cleanup().await;
                action_store.cleanup().await;
            }
        })
    }
}

// ... rest of the file (record_action, get_action, middlewares) ...

pub async fn record_action(
    state: &MiddlewareState,
    tenant_id: kernel_core::auth::TenantId,
    action_id: &str,
    status: ActionStatus,
) {
    state.action_store.record(tenant_id, action_id, status, state.config.action_ttl).await;
}

pub async fn get_action(
    state: &MiddlewareState,
    tenant_id: kernel_core::auth::TenantId,
    action_id: &str,
) -> Option<ActionRecord> {
    state.action_store.get(tenant_id, action_id).await
}

#[instrument(skip_all, fields(tenant_id, method, path))]
pub async fn idempotency_middleware(
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // ... same as before
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

    tracing::Span::current().record("tenant_id", &tenant_ctx.tenant_id().to_string());
    tracing::Span::current().record("method", method.as_str());
    tracing::Span::current().record("path", parts.uri.path());

    let path = parts.uri.path();
    let query = parts.uri.query();
    let canonical_target = canonicalize_request_target(path, query);

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

        match store
            .try_acquire(
                scope_key.clone(),
                body_hash.clone(),
                state.config.idempotency_ttl,
            )
            .await
        {
            None => {
                break;
            }
            Some(entry) => {
                match entry {
                    IdempotencyEntry::Completed(record) => {
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
                        continue;
                    }
                }
            }
        }
    }

    let response = next.run(req).await;

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

    store.release_inflight(&scope_key).await;
    Ok(response)
}

fn compute_body_hash(body: &[u8]) -> String {
    use std::fmt::Write;
    let result = digest(&SHA256, body);
    let bytes = result.as_ref();
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
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

pub async fn quota_middleware(req: Request<Body>, next: Next) -> Result<Response, Response> {
    let (parts, body) = req.into_parts();

    let tenant_ctx = match parts.extensions.get::<TenantContext>() {
        Some(ctx) => ctx,
        None => {
             warn!("Quota middleware missing TenantContext");
             return Ok(next.run(Request::from_parts(parts, body)).await);
        }
    };

    let state = match parts.extensions.get::<MiddlewareState>() {
        Some(s) => s,
        None => {
            error!("MiddlewareState missing");
            return Ok(next.run(Request::from_parts(parts, body)).await);
        }
    };

    #[cfg(debug_assertions)]
    {
        if parts.headers.contains_key("X-Mock-Quota-System") {
            let violation = QuotaViolation {
                layer: QuotaLayer::SystemHardLimit,
                retry_after_s: 100,
            };
            warn!("System Hard Limit exceeded (Mock)");
            return Err(violation_to_response(&violation));
        }

        if parts.headers.contains_key("X-Mock-Quota-Tenant") {
            let violation = QuotaViolation {
                layer: QuotaLayer::TenantBudget,
                retry_after_s: 5,
            };
            warn!("Tenant Budget exceeded (Mock)");
            return Err(violation_to_response(&violation));
        }

        if parts.headers.contains_key("X-Mock-Quota-Api") {
            let violation = QuotaViolation {
                layer: QuotaLayer::ApiRateLimit,
                retry_after_s: 60,
            };
            warn!("API Rate Limit exceeded (Mock)");
            return Err(violation_to_response(&violation));
        }
    }

    if let Err(v) = state.quota_store.check_and_update(tenant_ctx.tenant_id(), QuotaLayer::SystemHardLimit).await {
         return Err(violation_to_response(&v));
    }

    if let Err(v) = state.quota_store.check_and_update(tenant_ctx.tenant_id(), QuotaLayer::TenantBudget).await {
         return Err(violation_to_response(&v));
    }

    if let Err(v) = state.quota_store.check_and_update(tenant_ctx.tenant_id(), QuotaLayer::ApiRateLimit).await {
         return Err(violation_to_response(&v));
    }

    Ok(next.run(Request::from_parts(parts, body)).await)
}

fn response_not_cacheable_for_replay(headers: &HeaderMap, max_size: usize) -> bool {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > max_size)
}

fn violation_to_status(v: &QuotaViolation) -> StatusCode {
    match v.layer {
        QuotaLayer::SystemHardLimit => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::TOO_MANY_REQUESTS,
    }
}
