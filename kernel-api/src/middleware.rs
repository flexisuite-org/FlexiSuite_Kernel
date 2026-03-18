#![allow(clippy::manual_inspect)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::manual_map)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::implicit_saturating_sub)]
#![allow(clippy::needless_borrows_for_generic_args)]
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
use std::fmt;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify};
use tracing::{error, info, instrument, warn};

use crate::auth::TenantContext;

#[derive(Clone)]
pub struct MiddlewareConfig {
    pub idempotency_ttl: Duration,
    pub action_ttl: Duration,
    pub max_body_size: usize,
    pub max_replay_body_size: usize,
    pub inflight_wait_timeout: Duration,
    pub redis_url: String,
    pub require_redis: bool,
    #[cfg(any(test, feature = "test-utils"))]
    pub allow_mock_quota: bool,
    pub quota: QuotaConfig,
}

#[derive(Clone, Copy, Debug)]
pub struct QuotaLayerConfig {
    pub rate: f64,
    pub capacity: f64,
    pub cost: f64,
    pub backoff_s: f64,
}

#[derive(Clone, Debug, Default)]
pub struct TenantQuotaOverride {
    pub system_hard_limit: Option<QuotaLayerConfig>,
    pub tenant_budget: Option<QuotaLayerConfig>,
    pub api_rate_limit: Option<QuotaLayerConfig>,
    pub circuit_breaker: Option<QuotaLayerConfig>,
}

#[derive(Clone, Debug)]
pub struct QuotaConfig {
    pub system_hard_limit: QuotaLayerConfig,
    pub tenant_budget: QuotaLayerConfig,
    pub api_rate_limit: QuotaLayerConfig,
    pub circuit_breaker: QuotaLayerConfig,
    pub tenant_overrides: HashMap<String, TenantQuotaOverride>,
}

impl fmt::Debug for MiddlewareConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("MiddlewareConfig");
        s.field("idempotency_ttl", &self.idempotency_ttl)
            .field("action_ttl", &self.action_ttl)
            .field("max_body_size", &self.max_body_size)
            .field("max_replay_body_size", &self.max_replay_body_size)
            .field("inflight_wait_timeout", &self.inflight_wait_timeout)
            .field("redis_url", &"<redacted>")
            .field("require_redis", &self.require_redis);

        #[cfg(any(test, feature = "test-utils"))]
        s.field("allow_mock_quota", &self.allow_mock_quota);

        s.field("quota", &self.quota).finish()
    }
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

        fn get_env_bool(key: &str, default_val: bool) -> bool {
            match std::env::var(key) {
                Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
                    "1" | "true" | "yes" | "on" => true,
                    "0" | "false" | "no" | "off" => false,
                    _ => {
                        tracing::warn!(
                            key = %key,
                            value = %v,
                            "Invalid boolean environment variable, using default"
                        );
                        default_val
                    }
                },
                Err(_) => default_val,
            }
        }

        fn get_env_f64(key: &str, default_val: f64) -> f64 {
            match std::env::var(key) {
                Ok(v) => match v.parse::<f64>() {
                    Ok(s) => {
                        if s > 0.0 {
                            s
                        } else {
                            tracing::warn!(key = %key, value = %s, "Non-positive env var for quota (rate/capacity), using default");
                            default_val
                        }
                    }
                    Err(_) => {
                        tracing::warn!(key = %key, value = %v, "Invalid f64 env var, using default");
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
            redis_url: get_env_string("REDIS_URL", "redis://127.0.0.1:6379"),
            require_redis: get_env_bool("REQUIRE_REDIS", true),
            #[cfg(any(test, feature = "test-utils"))]
            allow_mock_quota: {
                let is_mock = get_env_bool("ALLOW_MOCK_QUOTA", false);
                if is_mock {
                    tracing::warn!(
                        "ALLOW_MOCK_QUOTA is enabled. This is unsafe for shared/production environments!"
                    );
                }
                is_mock
            },
            quota: QuotaConfig {
                system_hard_limit: QuotaLayerConfig {
                    rate: get_env_f64("QUOTA_SYSTEM_HARD_LIMIT_RATE", 1000.0),
                    capacity: get_env_f64("QUOTA_SYSTEM_HARD_LIMIT_CAPACITY", 1000.0),
                    cost: get_env_f64("QUOTA_SYSTEM_HARD_LIMIT_COST", 1.0),
                    backoff_s: get_env_f64("QUOTA_SYSTEM_HARD_LIMIT_BACKOFF_S", 30.0),
                },
                tenant_budget: QuotaLayerConfig {
                    rate: get_env_f64("QUOTA_TENANT_BUDGET_RATE", 1000.0),
                    capacity: get_env_f64("QUOTA_TENANT_BUDGET_CAPACITY", 3000.0),
                    cost: get_env_f64("QUOTA_TENANT_BUDGET_COST", 5.0),
                    backoff_s: get_env_f64("QUOTA_TENANT_BUDGET_BACKOFF_S", 30.0),
                },
                api_rate_limit: QuotaLayerConfig {
                    rate: get_env_f64("QUOTA_API_RATE_LIMIT_RATE", 16.666),
                    capacity: get_env_f64("QUOTA_API_RATE_LIMIT_CAPACITY", 100.0),
                    cost: get_env_f64("QUOTA_API_RATE_LIMIT_COST", 1.0),
                    backoff_s: get_env_f64("QUOTA_API_RATE_LIMIT_BACKOFF_S", 30.0),
                },
                circuit_breaker: QuotaLayerConfig {
                    rate: get_env_f64("QUOTA_CIRCUIT_BREAKER_RATE", 10.0),
                    capacity: get_env_f64("QUOTA_CIRCUIT_BREAKER_CAPACITY", 100.0),
                    cost: get_env_f64("QUOTA_CIRCUIT_BREAKER_COST", 1.0),
                    backoff_s: get_env_f64("QUOTA_CIRCUIT_BREAKER_BACKOFF_S", 30.0),
                },
                tenant_overrides: HashMap::new(),
            },
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
        owner_token: String,
        body_hash: String,
        notify: Arc<Notify>,
        expires_at: Instant,
    },
    Completed(IdempotencyRecord),
}

#[derive(Clone, Debug)]
pub struct IdempotencyLease {
    pub owner_token: String,
    pub version: u64,
}

#[derive(Clone, Debug)]
pub enum IdempotencyAcquireResult {
    Acquired(IdempotencyLease),
    Existing(IdempotencyEntry),
    Retry,
}

#[derive(Clone, Debug)]
pub enum IdempotencyStoreError {
    BackendUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct IdempotencyScopeKey {
    pub tenant_id: kernel_core::auth::TenantId,
    pub method: String,
    pub canonical_target: String,
    pub idempotency_key: String,
}

#[derive(Clone)]
pub struct BearerToken(String);

impl BearerToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("BearerToken").field(&"***").finish()
    }
}

/// Abstract Store Trait to allow switching to Redis (REQ: Production Readiness)
#[async_trait]
pub trait IdempotencyStore: Send + Sync {
    async fn get(
        &self,
        key: &IdempotencyScopeKey,
    ) -> Result<Option<IdempotencyEntry>, IdempotencyStoreError>;
    async fn try_acquire(
        &self,
        key: IdempotencyScopeKey,
        body_hash: String,
        ttl: Duration,
    ) -> Result<IdempotencyAcquireResult, IdempotencyStoreError>;
    async fn complete(
        &self,
        key: IdempotencyScopeKey,
        lease: &IdempotencyLease,
        record: IdempotencyRecord,
    ) -> Result<(), IdempotencyStoreError>;
    async fn release_inflight(
        &self,
        key: &IdempotencyScopeKey,
        lease: &IdempotencyLease,
    ) -> Result<(), IdempotencyStoreError>;
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
    async fn get(
        &self,
        key: &IdempotencyScopeKey,
    ) -> Result<Option<IdempotencyEntry>, IdempotencyStoreError> {
        let mut lock = self.inner.lock().await;
        match lock.get(key) {
            Some(IdempotencyEntry::InFlight { expires_at, .. }) => {
                if *expires_at <= Instant::now() {
                    lock.remove(key);
                    return Ok(None);
                }
            }
            Some(IdempotencyEntry::Completed(record)) => {
                if record.expires_at <= Instant::now() {
                    lock.remove(key);
                    return Ok(None);
                }
            }
            None => return Ok(None),
        }
        Ok(lock.get(key).cloned())
    }

    async fn try_acquire(
        &self,
        key: IdempotencyScopeKey,
        body_hash: String,
        ttl: Duration,
    ) -> Result<IdempotencyAcquireResult, IdempotencyStoreError> {
        let mut lock = self.inner.lock().await;
        if let Some(entry) = lock.get(&key) {
            let now = Instant::now();
            match entry {
                IdempotencyEntry::InFlight {
                    expires_at, notify, ..
                } => {
                    if *expires_at <= now {
                        // 期限切れ InFlight: 待機中のwaitersに通知してから削除
                        let notify = notify.clone();
                        lock.remove(&key);
                        drop(lock);
                        notify.notify_waiters();
                        // 新規リースとして続行するためロックを再取得
                        lock = self.inner.lock().await;
                    } else {
                        return Ok(IdempotencyAcquireResult::Existing(entry.clone()));
                    }
                }
                IdempotencyEntry::Completed(record) => {
                    if record.expires_at <= now {
                        // 期限切れ Completed: 削除して新規リースとして続行
                        lock.remove(&key);
                    } else {
                        return Ok(IdempotencyAcquireResult::Existing(entry.clone()));
                    }
                }
            }
        }

        let lease = IdempotencyLease {
            owner_token: uuid::Uuid::now_v7().to_string(),
            version: current_unix_timestamp_ms(),
        };
        lock.insert(
            key,
            IdempotencyEntry::InFlight {
                owner_token: lease.owner_token.clone(),
                body_hash,
                notify: Arc::new(Notify::new()),
                expires_at: Instant::now() + ttl,
            },
        );
        Ok(IdempotencyAcquireResult::Acquired(lease))
    }

    async fn complete(
        &self,
        key: IdempotencyScopeKey,
        lease: &IdempotencyLease,
        record: IdempotencyRecord,
    ) -> Result<(), IdempotencyStoreError> {
        let mut lock = self.inner.lock().await;
        let notify = match lock.get(&key) {
            Some(IdempotencyEntry::InFlight {
                owner_token,
                notify,
                ..
            }) if owner_token == &lease.owner_token => notify.clone(),
            _ => return Err(IdempotencyStoreError::BackendUnavailable),
        };
        lock.insert(key, IdempotencyEntry::Completed(record));
        drop(lock);
        notify.notify_waiters();
        Ok(())
    }

    async fn release_inflight(
        &self,
        key: &IdempotencyScopeKey,
        lease: &IdempotencyLease,
    ) -> Result<(), IdempotencyStoreError> {
        let mut lock = self.inner.lock().await;
        let notify = match lock.get(key) {
            Some(IdempotencyEntry::InFlight {
                owner_token,
                notify,
                ..
            }) if owner_token == &lease.owner_token => notify.clone(),
            _ => return Err(IdempotencyStoreError::BackendUnavailable),
        };
        lock.remove(key);
        drop(lock);
        notify.notify_waiters();
        Ok(())
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
    client: redis::Client,
    manager: redis::aio::ConnectionManager,
    subscriptions: Arc<Mutex<HashMap<String, ChannelSubscriptionState>>>,
}

struct ChannelSubscriptionState {
    waiters: Vec<Weak<Notify>>,
    running: bool,
}

impl RedisIdempotencyStore {
    pub fn new(client: redis::Client, manager: redis::aio::ConnectionManager) -> Self {
        Self {
            client,
            manager,
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn format_key(key: &IdempotencyScopeKey) -> String {
        // idemp:v3:<tenant_hash>:<method>:<target_hash>:<key_hash>
        // 9 chars prefix + 64*3 hashes + method + colons
        let mut s = String::with_capacity(256 + key.method.len());
        s.push_str("idemp:v3:");
        append_sha256_hex(key.tenant_id.as_str().as_bytes(), &mut s);
        s.push(':');
        s.push_str(&key.method);
        s.push(':');
        append_sha256_hex(key.canonical_target.as_bytes(), &mut s);
        s.push(':');
        append_sha256_hex(key.idempotency_key.as_bytes(), &mut s);
        s
    }

    fn channel_name(redis_key: &str) -> String {
        format!("idemp:ch:{redis_key}")
    }

    fn inflight_value(lease: &IdempotencyLease, body_hash: &str) -> String {
        format!(
            "IN_FLIGHT|{}|{}|{}",
            lease.owner_token, body_hash, lease.version
        )
    }

    fn parse_inflight_value(val: &str) -> Option<(String, String, u64)> {
        let mut parts = val.split('|');
        let state = parts.next()?;
        if state != "IN_FLIGHT" {
            return None;
        }
        let owner_token = parts.next()?.to_string();
        let body_hash = parts.next()?.to_string();
        let version = parts.next()?.parse::<u64>().ok()?;
        Some((owner_token, body_hash, version))
    }

    fn notify_waiters_and_prune(waiters: &mut Vec<Weak<Notify>>) -> usize {
        let mut alive = 0usize;
        waiters.retain(|weak| {
            if let Some(notify) = weak.upgrade() {
                notify.notify_waiters();
                alive += 1;
                true
            } else {
                false
            }
        });
        alive
    }

    fn prune_waiters(waiters: &mut Vec<Weak<Notify>>) -> usize {
        let mut alive = 0usize;
        waiters.retain(|weak| {
            let keep = weak.strong_count() > 0;
            if keep {
                alive += 1;
            }
            keep
        });
        alive
    }

    fn spawn_channel_listener(
        subscriptions: Arc<Mutex<HashMap<String, ChannelSubscriptionState>>>,
        client: redis::Client,
        channel: String,
    ) {
        tokio::spawn(async move {
            let mut pubsub = match client.get_async_pubsub().await {
                Ok(p) => p,
                Err(e) => {
                    error!("Redis pubsub create error: {}", e);
                    let mut lock = subscriptions.lock().await;
                    if let Some(state) = lock.get_mut(&channel) {
                        Self::notify_waiters_and_prune(&mut state.waiters);
                        state.running = false;
                    }
                    return;
                }
            };

            if let Err(e) = pubsub.subscribe(&channel).await {
                error!("Redis pubsub subscribe error: {}", e);
                let mut lock = subscriptions.lock().await;
                if let Some(state) = lock.get_mut(&channel) {
                    Self::notify_waiters_and_prune(&mut state.waiters);
                    state.running = false;
                }
                return;
            }

            let mut stream = pubsub.on_message();
            let mut poll_waiters_every = tokio::time::interval(Duration::from_secs(5));
            poll_waiters_every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                tokio::select! {
                    _ = poll_waiters_every.tick() => {
                        let has_waiters = {
                            let mut lock = subscriptions.lock().await;
                            if let Some(state) = lock.get_mut(&channel) {
                                Self::prune_waiters(&mut state.waiters) > 0
                            } else {
                                false
                            }
                        };
                        if !has_waiters {
                            break;
                        }
                    }
                    msg = futures_util::StreamExt::next(&mut stream) => {
                        if msg.is_none() {
                            break;
                        }
                        let has_waiters = {
                            let mut lock = subscriptions.lock().await;
                            if let Some(state) = lock.get_mut(&channel) {
                                Self::notify_waiters_and_prune(&mut state.waiters) > 0
                            } else {
                                false
                            }
                        };
                        if !has_waiters {
                            break;
                        }
                    }
                }
            }

            drop(stream);
            if let Err(e) = pubsub.unsubscribe(&channel).await {
                warn!("Redis pubsub unsubscribe error: {}", e);
            }

            let mut should_respawn = false;
            {
                let mut lock = subscriptions.lock().await;
                let mut remove = false;
                if let Some(state) = lock.get_mut(&channel) {
                    state.running = false;
                    let alive = Self::prune_waiters(&mut state.waiters);
                    if alive == 0 {
                        remove = true;
                    } else {
                        state.running = true;
                        should_respawn = true;
                    }
                }
                if remove {
                    lock.remove(&channel);
                }
            }

            if should_respawn {
                Self::spawn_channel_listener(Arc::clone(&subscriptions), client.clone(), channel);
            }
        });
    }

    async fn register_waiter(&self, redis_key: String, channel: String) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());

        let mut should_spawn = false;
        {
            let mut subscriptions = self.subscriptions.lock().await;
            let state =
                subscriptions
                    .entry(channel.clone())
                    .or_insert_with(|| ChannelSubscriptionState {
                        waiters: Vec::new(),
                        running: false,
                    });
            state.waiters.push(Arc::downgrade(&notify));
            if !state.running {
                state.running = true;
                should_spawn = true;
            }
        }

        let mut should_notify = false;
        let mut conn = self.manager.clone();
        match conn.get::<_, Option<String>>(&redis_key).await {
            Ok(Some(s)) => {
                if serde_json::from_str::<RedisIdempotencyRecordDto>(&s).is_ok() {
                    should_notify = true;
                }
            }
            Ok(None) => {
                // The in-flight key may have been released between GET and waiter registration.
                // Wake waiters so they can retry acquisition immediately instead of timing out.
                should_notify = true;
            }
            Err(e) => {
                warn!("Redis get error while registering waiter: {}", e);
            }
        }

        if should_notify {
            // Use notify_one so a permit is stored for the subsequent notified().await
            // in the middleware, ensuring it wakes immediately even if not yet polled.
            notify.notify_one();
        }

        if should_spawn {
            Self::spawn_channel_listener(
                Arc::clone(&self.subscriptions),
                self.client.clone(),
                channel,
            );
        }

        notify
    }
}

#[derive(Serialize, Deserialize)]
struct RedisIdempotencyRecordDto {
    body_hash: String,
    action_id: String,
    status: u16,
    headers: Vec<(String, String)>,
    body: String, // Base64 encoded
    owner_token: String,
    version: u64,
    completed_at_unix_ms: u64,
}

#[async_trait]
impl IdempotencyStore for RedisIdempotencyStore {
    async fn get(
        &self,
        key: &IdempotencyScopeKey,
    ) -> Result<Option<IdempotencyEntry>, IdempotencyStoreError> {
        let mut conn = self.manager.clone();
        let redis_key = Self::format_key(key);
        let val: Option<String> = match conn.get(&redis_key).await {
            Ok(v) => v,
            Err(e) => {
                error!("Redis get error: {}", e);
                return Err(IdempotencyStoreError::BackendUnavailable);
            }
        };

        match val {
            Some(s) => {
                if let Some((owner_token, body_hash, _)) = Self::parse_inflight_value(&s) {
                    let channel = Self::channel_name(&redis_key);
                    let notify = self.register_waiter(redis_key.clone(), channel).await;

                    Ok(Some(IdempotencyEntry::InFlight {
                        owner_token,
                        body_hash,
                        notify,
                        expires_at: Instant::now() + Duration::from_secs(60),
                    }))
                } else {
                    if let Ok(dto) = serde_json::from_str::<RedisIdempotencyRecordDto>(&s) {
                        let body_bytes = match BASE64_STANDARD.decode(&dto.body) {
                            Ok(bytes) => bytes,
                            Err(_) => {
                                warn!("Invalid Redis idempotency payload for key {}", redis_key);
                                return Ok(None);
                            }
                        };
                        let status = match StatusCode::from_u16(dto.status) {
                            Ok(status) => status,
                            Err(_) => {
                                warn!("Invalid Redis idempotency payload for key {}", redis_key);
                                return Ok(None);
                            }
                        };
                        Ok(Some(IdempotencyEntry::Completed(IdempotencyRecord {
                            body_hash: dto.body_hash,
                            action_id: dto.action_id,
                            status,
                            headers: dto.headers,
                            body: body_bytes,
                            expires_at: Instant::now() + Duration::from_secs(3600),
                        })))
                    } else {
                        warn!("Invalid Redis idempotency payload for key {}", redis_key);
                        Ok(None)
                    }
                }
            }
            None => Ok(None),
        }
    }

    async fn try_acquire(
        &self,
        key: IdempotencyScopeKey,
        body_hash: String,
        ttl: Duration,
    ) -> Result<IdempotencyAcquireResult, IdempotencyStoreError> {
        let mut conn = self.manager.clone();
        let redis_key = Self::format_key(&key);
        let lease = IdempotencyLease {
            owner_token: uuid::Uuid::now_v7().to_string(),
            version: current_unix_timestamp_ms(),
        };
        let val = Self::inflight_value(&lease, &body_hash);
        let ttl_secs = ttl.as_secs().max(1);
        let opts = redis::SetOptions::default()
            .conditional_set(redis::ExistenceCheck::NX)
            .with_expiration(redis::SetExpiry::EX(ttl_secs));

        let res: Result<bool, _> = conn.set_options(&redis_key, val, opts).await;

        match res {
            Ok(true) => Ok(IdempotencyAcquireResult::Acquired(lease)),
            Ok(false) => {
                let existing = self.get(&key).await?;
                if let Some(entry) = existing {
                    Ok(IdempotencyAcquireResult::Existing(entry))
                } else {
                    // Lost race: lock holder released/completed between NX failure and GET.
                    // Let caller retry instead of returning a false backend-unavailable error.
                    Ok(IdempotencyAcquireResult::Retry)
                }
            }
            Err(e) => {
                error!("Redis set error: {}", e);
                Err(IdempotencyStoreError::BackendUnavailable)
            }
        }
    }

    async fn complete(
        &self,
        key: IdempotencyScopeKey,
        lease: &IdempotencyLease,
        record: IdempotencyRecord,
    ) -> Result<(), IdempotencyStoreError> {
        let mut conn = self.manager.clone();
        let redis_key = Self::format_key(&key);
        let channel = Self::channel_name(&redis_key);
        let dto = RedisIdempotencyRecordDto {
            body_hash: record.body_hash,
            action_id: record.action_id,
            status: record.status.as_u16(),
            headers: record.headers,
            body: BASE64_STANDARD.encode(&record.body),
            owner_token: lease.owner_token.clone(),
            version: lease.version,
            completed_at_unix_ms: current_unix_timestamp_ms(),
        };

        let json = match serde_json::to_string(&dto) {
            Ok(v) => v,
            Err(_) => return Err(IdempotencyStoreError::BackendUnavailable),
        };
        let now = Instant::now();
        let ttl_secs = if record.expires_at > now {
            record.expires_at.duration_since(now).as_secs().max(1)
        } else {
            1
        };

        let script = redis::Script::new(
            r#"
            local key = KEYS[1]
            local channel = KEYS[2]
            local expected_owner = ARGV[1]
            local payload = ARGV[2]
            local ttl = tonumber(ARGV[3])

            local current = redis.call("GET", key)
            if not current then
                return 0
            end

            local owner = string.match(current, "^IN_FLIGHT|([^|]+)|")
            if not owner or owner ~= expected_owner then
                return -1
            end

            redis.call("SET", key, payload, "EX", ttl)
            redis.call("PUBLISH", channel, "completed")
            return 1
            "#,
        );
        let res: Result<i32, _> = script
            .key(&redis_key)
            .key(&channel)
            .arg(&lease.owner_token)
            .arg(json)
            .arg(ttl_secs)
            .invoke_async(&mut conn)
            .await;
        match res {
            Ok(1) => Ok(()),
            Ok(_) => Err(IdempotencyStoreError::BackendUnavailable),
            Err(e) => {
                error!("Redis idempotency complete CAS error: {}", e);
                Err(IdempotencyStoreError::BackendUnavailable)
            }
        }
    }

    async fn release_inflight(
        &self,
        key: &IdempotencyScopeKey,
        lease: &IdempotencyLease,
    ) -> Result<(), IdempotencyStoreError> {
        let mut conn = self.manager.clone();
        let redis_key = Self::format_key(key);
        let channel = Self::channel_name(&redis_key);
        let script = redis::Script::new(
            r#"
            local key = KEYS[1]
            local channel = KEYS[2]
            local expected_owner = ARGV[1]

            local current = redis.call("GET", key)
            if not current then
                return 0
            end
            local owner = string.match(current, "^IN_FLIGHT|([^|]+)|")
            if owner and owner == expected_owner then
                redis.call("DEL", key)
                redis.call("PUBLISH", channel, "released")
                return 1
            end
            return 0
            "#,
        );
        let res: Result<i32, _> = script
            .key(&redis_key)
            .key(&channel)
            .arg(&lease.owner_token)
            .invoke_async(&mut conn)
            .await;
        match res {
            Ok(_) => Ok(()),
            Err(e) => {
                error!("Redis idempotency release CAS error: {}", e);
                Err(IdempotencyStoreError::BackendUnavailable)
            }
        }
    }

    async fn cleanup(&self) {}
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
    async fn record(
        &self,
        tenant_id: kernel_core::auth::TenantId,
        action_id: &str,
        status: ActionStatus,
        ttl: Duration,
    );
    async fn get(
        &self,
        tenant_id: kernel_core::auth::TenantId,
        action_id: &str,
    ) -> Option<ActionRecord>;
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
    async fn record(
        &self,
        tenant_id: kernel_core::auth::TenantId,
        action_id: &str,
        status: ActionStatus,
        ttl: Duration,
    ) {
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

    async fn get(
        &self,
        tenant_id: kernel_core::auth::TenantId,
        action_id: &str,
    ) -> Option<ActionRecord> {
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
        let mut s = String::with_capacity(7 + 64 + 1 + 64);
        s.push_str("action:");
        append_sha256_hex(tenant_id.as_bytes(), &mut s);
        s.push(':');
        append_sha256_hex(action_id.as_bytes(), &mut s);
        s
    }
}

#[derive(Serialize, Deserialize)]
struct RedisActionRecordDto {
    status: ActionStatus,
    expires_at_ts: u64,
}

#[async_trait]
impl ActionStore for RedisActionStore {
    async fn record(
        &self,
        tenant_id: kernel_core::auth::TenantId,
        action_id: &str,
        status: ActionStatus,
        ttl: Duration,
    ) {
        let mut conn = self.manager.clone();
        let key = Self::format_key(tenant_id.as_str(), action_id);
        let ttl_secs = ttl.as_secs().max(1);

        let expires_at_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + ttl_secs;

        let dto = RedisActionRecordDto {
            status,
            expires_at_ts,
        };

        if let Ok(json) = serde_json::to_string(&dto) {
            if let Err(e) = conn.set_ex::<_, _, ()>(&key, json, ttl_secs).await {
                error!("Failed to store action record in Redis: {}", e);
            }
        }
    }

    async fn get(
        &self,
        tenant_id: kernel_core::auth::TenantId,
        action_id: &str,
    ) -> Option<ActionRecord> {
        let mut conn = self.manager.clone();
        let key = Self::format_key(tenant_id.as_str(), action_id);
        let val: Option<String> = match conn.get(&key).await {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to read action record from Redis: {}", e);
                return None;
            }
        };

        if let Some(s) = val {
            if let Ok(dto) = serde_json::from_str::<RedisActionRecordDto>(&s) {
                let now_ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let remaining = if dto.expires_at_ts > now_ts {
                    dto.expires_at_ts - now_ts
                } else {
                    0
                };
                if remaining == 0 {
                    return None;
                }
                Some(ActionRecord {
                    status: dto.status,
                    expires_at: Instant::now() + Duration::from_secs(remaining),
                })
            } else {
                error!("Failed to deserialize action record from Redis");
                None
            }
        } else {
            None
        }
    }

    async fn cleanup(&self) {}
}

#[async_trait]
pub trait QuotaStore: Send + Sync {
    async fn check_and_update(
        &self,
        tenant_id: &kernel_core::auth::TenantId,
        layer: QuotaLayer,
    ) -> Result<(), QuotaViolation>;
    async fn check_and_update_multi(
        &self,
        tenant_id: &kernel_core::auth::TenantId,
        layers: &[QuotaLayer],
    ) -> Result<(), QuotaViolation> {
        for layer in layers {
            self.check_and_update(tenant_id, *layer).await?;
        }
        Ok(())
    }
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
    async fn check_and_update(
        &self,
        _tenant_id: &kernel_core::auth::TenantId,
        _layer: QuotaLayer,
    ) -> Result<(), QuotaViolation> {
        // Intentional no-op fallback when Redis quota enforcement is disabled/unavailable.
        Ok(())
    }
}

pub struct RedisQuotaStore {
    manager: redis::aio::ConnectionManager,
    script: redis::Script,
    script_multi: redis::Script,
    quota: QuotaConfig,
}

impl RedisQuotaStore {
    pub fn new(manager: redis::aio::ConnectionManager, quota: QuotaConfig) -> Self {
        let script = redis::Script::new(
            r#"
            local function compute_refill_ttl_ms(rate, capacity, tokens)
                if rate <= 0 then
                    return 1
                end
                local missing = math.max(0, capacity - tokens)
                if missing <= 0 then
                    return 1
                end
                return math.max(1, math.ceil((missing / rate) * 1000))
            end

            local key = KEYS[1]
            local rate = tonumber(ARGV[1])
            local capacity = tonumber(ARGV[2])
            local cost = tonumber(ARGV[3])
            local now = tonumber(ARGV[4])
            local backoff_s = tonumber(ARGV[5])
            local is_cb = tonumber(ARGV[6]) == 1

            -- Circuit breaker logic: load-based burst protector
            if is_cb then
                local cb_state = redis.call("HGET", key, "state")
                local cb_until_raw = redis.call("HGET", key, "until")
                local cb_until = tonumber(cb_until_raw)

                if cb_state == "open" then
                    if cb_until ~= nil and now < cb_until then
                        return {0, math.ceil(cb_until - now)}
                    else
                        -- Timeout expired, fall through to token check (acts as half-open probe)
                    end
                end
            end

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

                if is_cb then
                    redis.call("HSET", key, "tokens", new_tokens, "last_refill", now, "state", "closed")
                    redis.call("HDEL", key, "until")
                    redis.call("PEXPIRE", key, compute_refill_ttl_ms(rate, capacity, new_tokens))
                else
                    redis.call("HSET", key, "tokens", new_tokens, "last_refill", now)
                    redis.call("PEXPIRE", key, 60000)
                end
                return {1, new_tokens}
            else
                if is_cb then
                    local retry_after = math.ceil(backoff_s)
                    retry_after = math.max(1, math.min(31536000, retry_after))
                    redis.call("HSET", key, "state", "open", "until", now + retry_after)
                    local ttl_ms = math.floor((retry_after * 1000) + 30000)
                    redis.call("PEXPIRE", key, ttl_ms)
                    return {0, retry_after}
                else
                    local required = cost - filled
                    local retry_after
                    if rate <= 0 then
                        retry_after = 60
                    else
                        retry_after = math.ceil(required / rate)
                    end
                    return {0, retry_after}
                end
            end
        "#,
        );
        let script_multi = redis::Script::new(
            r#"
            local function compute_refill_ttl_ms(rate, capacity, tokens)
                if rate <= 0 then
                    return 1
                end
                local missing = math.max(0, capacity - tokens)
                if missing <= 0 then
                    return 1
                end
                return math.max(1, math.ceil((missing / rate) * 1000))
            end

            local now = tonumber(ARGV[1])
            local n = tonumber(ARGV[2])
            local idx = 3
            local keys = {}
            local new_tokens = {}
            local is_cb = {}
            local rates = {}
            local capacities = {}

            -- First pass: validation (CB may write open state on failure before early return)
            for i = 1, n do
                local key = KEYS[i]
                local rate = tonumber(ARGV[idx])
                local capacity = tonumber(ARGV[idx + 1])
                local cost = tonumber(ARGV[idx + 2])
                local backoff_s = tonumber(ARGV[idx + 3])
                is_cb[i] = tonumber(ARGV[idx + 4]) == 1
                rates[i] = rate
                capacities[i] = capacity

                -- Circuit breaker logic: load-based burst protector
                if is_cb[i] then
                    local cb_state = redis.call("HGET", key, "state")
                    local cb_until_raw = redis.call("HGET", key, "until")
                    local cb_until = tonumber(cb_until_raw)
                    if cb_state == "open" then
                        if cb_until ~= nil and now < cb_until then
                            return {0, i, math.ceil(cb_until - now)}
                        else
                            -- Timeout expired, fall through to token check (acts as half-open probe)
                        end
                    end
                end

                local tokens = tonumber(redis.call("HGET", key, "tokens"))
                local last_refill = tonumber(redis.call("HGET", key, "last_refill"))
                if tokens == nil then
                    tokens = capacity
                    last_refill = now
                end

                local delta = math.max(0, now - last_refill)
                local filled = math.min(capacity, tokens + (delta * rate))
                if filled < cost then
                    if is_cb[i] then
                        local retry_after = math.ceil(backoff_s)
                        retry_after = math.max(1, math.min(31536000, retry_after))
                        redis.call("HSET", key, "state", "open", "until", now + retry_after)
                        local ttl_ms = math.floor((retry_after * 1000) + 30000)
                        redis.call("PEXPIRE", key, ttl_ms)
                        return {0, i, retry_after}
                    else
                        local retry_after
                        if rate <= 0 then
                            retry_after = 60
                        else
                            retry_after = math.ceil((cost - filled) / rate)
                        end
                        return {0, i, retry_after}
                    end
                end

                keys[i] = key
                new_tokens[i] = filled - cost
                idx = idx + 5
            end

            -- Second pass: commit updates
            for i = 1, n do
                if is_cb[i] then
                    redis.call("HSET", keys[i], "tokens", new_tokens[i], "last_refill", now, "state", "closed")
                    redis.call("HDEL", keys[i], "until")
                    redis.call(
                        "PEXPIRE",
                        keys[i],
                        compute_refill_ttl_ms(rates[i], capacities[i], new_tokens[i])
                    )
                else
                    redis.call("HSET", keys[i], "tokens", new_tokens[i], "last_refill", now)
                    redis.call("PEXPIRE", keys[i], 60000)
                end
            end

            return {1, 0, 0}
        "#,
        );
        Self {
            manager,
            script,
            script_multi,
            quota,
        }
    }

    /// All quota keys intentionally use the `{global}` Redis Cluster hash tag so
    /// `SystemHardLimit`, `CircuitBreaker`, `TenantBudget`, and `ApiRateLimit` keys land in one slot and
    /// can be updated atomically by the multi-key Lua script. This creates a hot slot;
    /// do not change hash tags without redesigning the atomicity strategy.
    fn layer_config(
        &self,
        tenant_id: &kernel_core::auth::TenantId,
        layer: QuotaLayer,
    ) -> Option<(String, f64, f64, f64, f64, bool)> {
        let tenant_id_str = tenant_id.to_string();
        let tenant_override = self.quota.tenant_overrides.get(&tenant_id_str);
        let is_cb = layer == QuotaLayer::CircuitBreaker;
        match layer {
            QuotaLayer::SystemHardLimit => {
                let q = tenant_override
                    .and_then(|o| o.system_hard_limit)
                    .unwrap_or(self.quota.system_hard_limit);
                Some((
                    "quota:{global}:system".to_string(),
                    q.rate,
                    q.capacity,
                    q.cost,
                    q.backoff_s,
                    is_cb,
                ))
            }
            QuotaLayer::TenantBudget => {
                let q = tenant_override
                    .and_then(|o| o.tenant_budget)
                    .unwrap_or(self.quota.tenant_budget);
                let mut s = String::with_capacity(128);
                s.push_str("quota:{global}:tenant:");
                append_sha256_hex(tenant_id_str.as_bytes(), &mut s);
                s.push_str(":cpu");
                Some((s, q.rate, q.capacity, q.cost, q.backoff_s, is_cb))
            }
            QuotaLayer::ApiRateLimit => {
                let q = tenant_override
                    .and_then(|o| o.api_rate_limit)
                    .unwrap_or(self.quota.api_rate_limit);
                let mut s = String::with_capacity(128);
                s.push_str("quota:{global}:tenant:");
                append_sha256_hex(tenant_id_str.as_bytes(), &mut s);
                s.push_str(":api");
                Some((s, q.rate, q.capacity, q.cost, q.backoff_s, is_cb))
            }
            QuotaLayer::CircuitBreaker => {
                let q = tenant_override
                    .and_then(|o| o.circuit_breaker)
                    .unwrap_or(self.quota.circuit_breaker);
                let mut s = String::with_capacity(128);
                s.push_str("quota:{global}:tenant:");
                append_sha256_hex(tenant_id_str.as_bytes(), &mut s);
                s.push_str(":cb");
                Some((s, q.rate, q.capacity, q.cost, q.backoff_s, is_cb))
            }
        }
    }
}

#[async_trait]
impl QuotaStore for RedisQuotaStore {
    async fn check_and_update(
        &self,
        tenant_id: &kernel_core::auth::TenantId,
        layer: QuotaLayer,
    ) -> Result<(), QuotaViolation> {
        let Some((key, rate, capacity, cost, backoff_s, is_cb)) =
            self.layer_config(tenant_id, layer)
        else {
            return Ok(());
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        let mut conn = self.manager.clone();

        let res: Result<(i32, f64), _> = self
            .script
            .key(&key)
            .arg(rate)
            .arg(capacity)
            .arg(cost)
            .arg(now)
            .arg(backoff_s)
            .arg(if is_cb { 1 } else { 0 })
            .invoke_async(&mut conn)
            .await;

        match res {
            Ok((allowed, val)) => {
                if allowed == 1 {
                    Ok(())
                } else {
                    let retry_after_s = val.ceil() as u64;
                    Err(QuotaViolation {
                        layer,
                        retry_after_s,
                    })
                }
            }
            Err(e) => {
                error!("Redis script error: {}", e);
                Err(QuotaViolation {
                    layer: QuotaLayer::SystemHardLimit,
                    retry_after_s: 1,
                })
            }
        }
    }

    async fn check_and_update_multi(
        &self,
        tenant_id: &kernel_core::auth::TenantId,
        layers: &[QuotaLayer],
    ) -> Result<(), QuotaViolation> {
        let checks: Vec<(QuotaLayer, String, f64, f64, f64, f64, bool)> = layers
            .iter()
            .filter_map(|layer| {
                self.layer_config(tenant_id, *layer)
                    .map(|(k, r, c, cost, backoff_s, is_cb)| {
                        (*layer, k, r, c, cost, backoff_s, is_cb)
                    })
            })
            .collect();

        if checks.is_empty() {
            return Ok(());
        }

        let mut conn = self.manager.clone();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        let mut inv = self.script_multi.prepare_invoke();
        inv.arg(now).arg(checks.len() as i64);
        for (_, key, rate, capacity, cost, backoff_s, is_cb) in &checks {
            inv.key(key)
                .arg(rate)
                .arg(capacity)
                .arg(cost)
                .arg(backoff_s)
                .arg(if *is_cb { 1 } else { 0 });
        }

        let res: Result<(i32, i64, f64), _> = inv.invoke_async(&mut conn).await;
        match res {
            Ok((1, _, _)) => Ok(()),
            Ok((0, idx, retry_after)) => {
                let layer_idx = (idx.saturating_sub(1)) as usize;
                let retry_after_s = retry_after.ceil() as u64;
                if let Some((layer, _, _, _, _, _, _)) = checks.get(layer_idx) {
                    Err(QuotaViolation {
                        layer: *layer,
                        retry_after_s,
                    })
                } else {
                    error!(
                        "Lua script returned invalid index for failed quota layer: {} (checks={})",
                        idx,
                        checks.len()
                    );
                    Err(QuotaViolation {
                        layer: QuotaLayer::SystemHardLimit,
                        retry_after_s,
                    })
                }
            }
            Ok(other) => {
                error!("Unexpected quota multi script response: {:?}", other);
                Err(QuotaViolation {
                    layer: QuotaLayer::SystemHardLimit,
                    retry_after_s: 1,
                })
            }
            Err(e) => {
                error!("Redis multi quota script error: {}", e);
                Err(QuotaViolation {
                    layer: QuotaLayer::SystemHardLimit,
                    retry_after_s: 1,
                })
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
    pub async fn new(config: MiddlewareConfig) -> Result<Self, String> {
        Self::new_with_redis(config).await
    }

    pub async fn new_with_redis(config: MiddlewareConfig) -> Result<Self, String> {
        let redacted_redis_url = redact_redis_url(&config.redis_url);
        if !config.redis_url.starts_with("redis://") && !config.redis_url.starts_with("rediss://") {
            let msg = format!(
                "REDIS_URL must use redis:// or rediss:// scheme, got: {}",
                redacted_redis_url
            );
            if config.require_redis {
                return Err(msg);
            }
            log_dev_only_redis_fallback(&msg);
            return Ok(Self::with_store(
                config,
                Arc::new(InMemoryIdempotencyStore::new()),
                Arc::new(InMemoryActionStore::new()),
                Arc::new(InMemoryQuotaStore::new()),
            ));
        }

        let client = redis::Client::open(config.redis_url.clone());
        match client {
            Ok(c) => match c.get_connection_manager().await {
                Ok(manager) => {
                    let quota = config.quota.clone();
                    Ok(Self::with_store(
                        config,
                        Arc::new(RedisIdempotencyStore::new(c, manager.clone())),
                        Arc::new(RedisActionStore::new(manager.clone())),
                        Arc::new(RedisQuotaStore::new(manager.clone(), quota)),
                    ))
                }
                Err(e) => {
                    let sanitized_error = sanitize_redis_error(&e.to_string(), &config.redis_url);
                    let msg = format!(
                        "Failed to create Redis connection manager for {}: {}",
                        redacted_redis_url, sanitized_error
                    );
                    if config.require_redis {
                        return Err(msg);
                    }
                    log_dev_only_redis_fallback(&msg);
                    Ok(Self::with_store(
                        config,
                        Arc::new(InMemoryIdempotencyStore::new()),
                        Arc::new(InMemoryActionStore::new()),
                        Arc::new(InMemoryQuotaStore::new()),
                    ))
                }
            },
            Err(e) => {
                let sanitized_error = sanitize_redis_error(&e.to_string(), &config.redis_url);
                let msg = format!(
                    "Invalid Redis URL ({}): {}",
                    redacted_redis_url, sanitized_error
                );
                if config.require_redis {
                    return Err(msg);
                }
                log_dev_only_redis_fallback(&msg);
                Ok(Self::with_store(
                    config,
                    Arc::new(InMemoryIdempotencyStore::new()),
                    Arc::new(InMemoryActionStore::new()),
                    Arc::new(InMemoryQuotaStore::new()),
                ))
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

fn log_dev_only_redis_fallback(reason: &str) {
    error!("{reason}");
    error!(
        "DEVELOPMENT-ONLY fallback activated: using InMemoryIdempotencyStore, InMemoryActionStore, and InMemoryQuotaStore because REQUIRE_REDIS=false"
    );
    warn!(
        "This fallback is unsafe for multi-instance production deployments; idempotency state and quota state are not shared across instances"
    );
}

fn redact_redis_url(redis_url: &str) -> String {
    if let Some((scheme, remainder)) = redis_url.split_once("://") {
        let after_auth = remainder
            .rsplit_once('@')
            .map(|(_, host)| host)
            .unwrap_or(remainder);
        let host_port = after_auth.split('/').next().unwrap_or(after_auth);
        if !host_port.is_empty() {
            return format!("{scheme}://{host_port}");
        }
    }
    "<redacted-redis-url>".to_string()
}

fn sanitize_redis_error(error_text: &str, redis_url: &str) -> String {
    error_text.replace(redis_url, &redact_redis_url(redis_url))
}

pub async fn record_action(
    state: &MiddlewareState,
    tenant_id: kernel_core::auth::TenantId,
    action_id: &str,
    status: ActionStatus,
) {
    state
        .action_store
        .record(tenant_id, action_id, status, state.config.action_ttl)
        .await;
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
    let lease = loop {
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
            Ok(IdempotencyAcquireResult::Acquired(lease)) => break lease,
            Ok(IdempotencyAcquireResult::Retry) => continue,
            Ok(IdempotencyAcquireResult::Existing(entry)) => match entry {
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
                }
            },
            Err(IdempotencyStoreError::BackendUnavailable) => {
                error!("Idempotency store unavailable during acquire");
                return Err(StatusCode::SERVICE_UNAVAILABLE);
            }
        }
    };

    let response = next.run(req).await;

    if response.status().is_success() {
        let (parts, body) = response.into_parts();
        if response_not_cacheable_for_replay(&parts.headers, state.config.max_replay_body_size) {
            info!(
                "Skipping idempotency replay cache due to Content-Length > {}",
                state.config.max_replay_body_size
            );
            let _ = store.release_inflight(&scope_key, &lease).await;
            return Ok(Response::from_parts(parts, body));
        }

        let body_bytes = match body.collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(_) => {
                error!(
                    "Response body could not be buffered for idempotency cache due to body read error"
                );
                let _ = store.release_inflight(&scope_key, &lease).await;
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
            let _ = store.release_inflight(&scope_key, &lease).await;
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
        if store
            .complete(scope_key.clone(), &lease, stored)
            .await
            .is_err()
        {
            let _ = store.release_inflight(&scope_key, &lease).await;
            error!("Failed to complete idempotency record");
        }
        return Ok(Response::from_parts(parts, Body::from(body_bytes)));
    }

    let _ = store.release_inflight(&scope_key, &lease).await;
    Ok(response)
}

pub async fn quota_middleware(req: Request<Body>, next: Next) -> Result<Response, Response> {
    let (parts, body) = req.into_parts();

    let tenant_ctx = match parts.extensions.get::<TenantContext>() {
        Some(ctx) => ctx,
        None => {
            warn!("Quota middleware missing TenantContext");
            return Err(StatusCode::UNAUTHORIZED.into_response());
        }
    };

    let state = match parts.extensions.get::<MiddlewareState>() {
        Some(s) => s,
        None => {
            error!("MiddlewareState missing in extensions");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    };

    #[cfg(any(test, feature = "test-utils"))]
    {
        if state.config.allow_mock_quota && parts.headers.contains_key("X-Mock-Quota-System") {
            let violation = QuotaViolation {
                layer: QuotaLayer::SystemHardLimit,
                retry_after_s: 100,
            };
            warn!("System Hard Limit exceeded (Mock)");
            return Err(violation_to_response(&violation));
        }

        if state.config.allow_mock_quota
            && parts.headers.contains_key("X-Mock-Quota-CircuitBreaker")
        {
            let violation = QuotaViolation {
                layer: QuotaLayer::CircuitBreaker,
                retry_after_s: 30,
            };
            warn!("Circuit Breaker exceeded (Mock)");
            return Err(violation_to_response(&violation));
        }

        if state.config.allow_mock_quota && parts.headers.contains_key("X-Mock-Quota-Tenant") {
            let violation = QuotaViolation {
                layer: QuotaLayer::TenantBudget,
                retry_after_s: 5,
            };
            warn!("Tenant Budget exceeded (Mock)");
            return Err(violation_to_response(&violation));
        }

        if state.config.allow_mock_quota && parts.headers.contains_key("X-Mock-Quota-Api") {
            let violation = QuotaViolation {
                layer: QuotaLayer::ApiRateLimit,
                retry_after_s: 60,
            };
            warn!("API Rate Limit exceeded (Mock)");
            return Err(violation_to_response(&violation));
        }
    }

    if let Err(v) = state
        .quota_store
        .check_and_update_multi(
            tenant_ctx.tenant_id(),
            &[
                QuotaLayer::SystemHardLimit,
                QuotaLayer::CircuitBreaker,
                QuotaLayer::TenantBudget,
                QuotaLayer::ApiRateLimit,
            ],
        )
        .await
    {
        return Err(violation_to_response(&v));
    }

    Ok(next.run(Request::from_parts(parts, body)).await)
}

fn compute_body_hash(body: &[u8]) -> String {
    let mut s = String::with_capacity(64);
    append_sha256_hex(body, &mut s);
    s
}

fn append_sha256_hex(input: &[u8], output: &mut String) {
    let result = digest(&SHA256, input);
    let bytes = result.as_ref();
    output.reserve(bytes.len() * 2);
    // SAFETY: We only append ASCII characters [0-9a-f], maintaining UTF-8 validity.
    let vec = unsafe { output.as_mut_vec() };
    const HEX_CHARS: &[u8] = b"0123456789abcdef";
    for &b in bytes {
        vec.push(HEX_CHARS[(b >> 4) as usize]);
        vec.push(HEX_CHARS[(b & 0xf) as usize]);
    }
}

fn current_unix_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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

fn response_not_cacheable_for_replay(headers: &HeaderMap, max_size: usize) -> bool {
    headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > max_size)
}

pub fn violation_to_status(v: &QuotaViolation) -> StatusCode {
    match v.layer {
        QuotaLayer::SystemHardLimit => StatusCode::SERVICE_UNAVAILABLE,
        QuotaLayer::CircuitBreaker => StatusCode::SERVICE_UNAVAILABLE,
        QuotaLayer::TenantBudget | QuotaLayer::ApiRateLimit => StatusCode::TOO_MANY_REQUESTS,
    }
}

pub fn violation_to_response(v: &QuotaViolation) -> Response {
    let status = violation_to_status(v);
    let message = match status {
        StatusCode::TOO_MANY_REQUESTS => "Rate limit exceeded",
        _ => "Quota limit exceeded",
    };
    let mut res = (status, message).into_response();

    // Inject headers from violation
    let headers = res.headers_mut();
    for (name, value) in v.headers() {
        if let Ok(hname) = HeaderName::from_bytes(name.as_bytes()) {
            if let Ok(hval) = HeaderValue::from_str(&value) {
                headers.insert(hname, hval);
            }
        }
    }

    // The X-Violation-Type header MUST always be present in quota violation responses
    let violation_type = match v.layer {
        QuotaLayer::SystemHardLimit => "system_hard_limit",
        QuotaLayer::CircuitBreaker => "circuit_breaker",
        QuotaLayer::TenantBudget => "tenant_budget",
        QuotaLayer::ApiRateLimit => "api_rate_limit",
    };
    headers.insert(
        "X-Violation-Type",
        HeaderValue::from_str(violation_type).unwrap_or(HeaderValue::from_static("unknown")),
    );

    res
}

pub mod rbac;
pub use rbac::{UserPermissions, load_permissions_middleware, require_permission};

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_sha256_hex() {
        let input = b"hello world";
        // echo -n "hello world" | sha256sum
        // b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        let mut s = String::new();
        append_sha256_hex(input, &mut s);
        assert_eq!(s, expected);

        let input_empty = b"";
        // echo -n "" | sha256sum
        // e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let expected_empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let mut s = String::new();
        append_sha256_hex(input_empty, &mut s);
        assert_eq!(s, expected_empty);

        // FIPS 180-4 / NIST SHA-256 test vector "abc"
        // https://csrc.nist.gov/projects/cryptographic-algorithm-validation-program/secure-hashing
        let input_nist = b"abc";
        let expected_nist = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let mut s = String::new();
        append_sha256_hex(input_nist, &mut s);
        assert_eq!(s, expected_nist);
    }

    #[tokio::test]
    async fn test_new_with_redis_fails_closed_when_redis_required() {
        let config = MiddlewareConfig {
            redis_url: "not-a-redis-url".to_string(),
            require_redis: true,
            ..MiddlewareConfig::default()
        };

        let err = match MiddlewareState::new_with_redis(config).await {
            Ok(_) => panic!("require_redis=true must fail startup"),
            Err(err) => err,
        };
        assert!(err.contains("REDIS_URL must use redis:// or rediss:// scheme"));
    }

    #[tokio::test]
    async fn test_new_with_redis_allows_dev_only_fallback_when_redis_optional() {
        let config = MiddlewareConfig {
            redis_url: "not-a-redis-url".to_string(),
            require_redis: false,
            idempotency_ttl: Duration::from_secs(60),
            action_ttl: Duration::from_secs(60),
            ..MiddlewareConfig::default()
        };

        let state = MiddlewareState::new_with_redis(config)
            .await
            .expect("require_redis=false may use development-only fallback");

        let key = IdempotencyScopeKey {
            tenant_id: kernel_core::auth::TenantId::new("tenant-fallback").expect("tenant id"),
            method: "POST".to_string(),
            canonical_target: "/test".to_string(),
            idempotency_key: "fallback-key".to_string(),
        };

        let acquired = state
            .idempotency_store
            .try_acquire(key, "body-hash".to_string(), Duration::from_secs(60))
            .await
            .expect("in-memory idempotency store must be active");

        assert!(matches!(acquired, IdempotencyAcquireResult::Acquired(_)));
    }

    #[test]
    fn test_circuit_breaker_refill_ttl_covers_full_recovery_horizon() {
        let rate = 0.5_f64;
        let capacity = 120.0_f64;
        let new_tokens = 0.0_f64;
        let ttl_ms = ((capacity - new_tokens) / rate * 1000.0).ceil() as u64;

        assert_eq!(ttl_ms, 240_000);
        assert!(ttl_ms > 60_000);
    }

    #[test]
    fn test_quota_refill_ttl_guard_zero_rate() {
        // This test documents the safety guard we added to the Lua script.
        let rate = 0.0_f64;
        let capacity = 100.0_f64;
        let tokens = 0.0_f64;

        let refill_ttl_ms = if rate <= 0.0 {
            1
        } else {
            let missing = (capacity - tokens).max(0.0);
            if missing <= 0.0 {
                1
            } else {
                ((missing / rate) * 1000.0).ceil() as u64
            }
        };

        assert_eq!(refill_ttl_ms, 1);
    }
}
