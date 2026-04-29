use crate::auth_context::TenantId;
use crate::event::{
    Delivery, EventEnvelope, EventError, ReliableConsumer, RetryPolicy, SHARD_COUNT,
    validate_stream_key,
};
use async_trait::async_trait;
use moka::future::Cache;
use redis::aio::ConnectionManager;
use redis::streams::{
    StreamAutoClaimOptions, StreamAutoClaimReply, StreamClaimReply, StreamId, StreamReadOptions,
    StreamReadReply,
};
use redis::{AsyncCommands, Client, RedisError};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering as AtomicOrdering},
};
use std::time::Duration;
#[cfg(test)]
use testcontainers::{ImageExt, runners::AsyncRunner};
#[cfg(test)]
use testcontainers_modules::redis::{REDIS_PORT, Redis};
use tracing::{instrument, warn};

#[derive(Clone)]
pub struct RedisConsumer {
    connection_manager: ConnectionManager,
    block_timeout: Duration,
    next_poll_start_shard: Arc<AtomicUsize>,
    ensured_consumer_groups: Cache<String, ()>,
    /// The Redis stream ID to use when creating a missing consumer group during NOGROUP recovery.
    /// "0" replays the full stream history (at-least-once), "$" starts with new messages only.
    nogroup_recovery_start_id: String,
}

impl RedisConsumer {
    const DEFAULT_BLOCK_TIMEOUT: Duration = Duration::from_secs(1);

    pub async fn new(client: Client) -> Result<Self, EventError> {
        let connection_manager = client.get_connection_manager().await.map_err(|e| {
            EventError::Consumer(format!("failed to create connection manager: {e}"))
        })?;
        Ok(Self {
            connection_manager,
            block_timeout: Self::DEFAULT_BLOCK_TIMEOUT,
            next_poll_start_shard: Arc::new(AtomicUsize::new(0)),
            ensured_consumer_groups: Cache::builder()
                .max_capacity(2048)
                .time_to_live(Duration::from_secs(600)) // 10 minutes TTL to detect externally deleted groups
                .build(),
            nogroup_recovery_start_id: "0".to_string(),
        })
    }

    pub async fn new_with_config(
        client: Client,
        block_timeout: Duration,
        nogroup_recovery_start_id: Option<String>,
    ) -> Result<Self, EventError> {
        Self::validate_block_timeout(block_timeout)?;
        let connection_manager = client.get_connection_manager().await.map_err(|e| {
            EventError::Consumer(format!("failed to create connection manager: {e}"))
        })?;
        Ok(Self {
            connection_manager,
            block_timeout,
            next_poll_start_shard: Arc::new(AtomicUsize::new(0)),
            ensured_consumer_groups: Cache::builder()
                .max_capacity(2048)
                .time_to_live(Duration::from_secs(600))
                .build(),
            nogroup_recovery_start_id: nogroup_recovery_start_id.unwrap_or_else(|| "0".to_string()),
        })
    }

    fn validate_stream_base(stream_base: &str) -> Result<(), EventError> {
        if stream_base.is_empty() || stream_base.contains(':') {
            return Err(EventError::Consumer(format!(
                "invalid stream_base: '{stream_base}'. Must not be empty or contain ':'"
            )));
        }
        Ok(())
    }

    fn validate_block_timeout(block_timeout: Duration) -> Result<(), EventError> {
        if block_timeout.is_zero() {
            return Err(EventError::Consumer(
                "block_timeout must be greater than zero to avoid indefinite blocking".to_string(),
            ));
        }
        Ok(())
    }

    fn stream_key_for_shard(tenant_id: &TenantId, stream_base: &str, shard: u64) -> String {
        format!("{}:{}:{}", tenant_id, stream_base, shard)
    }

    fn stream_keys_for_tenant(tenant_id: &TenantId, stream_base: &str) -> Vec<String> {
        (0..SHARD_COUNT)
            .map(|shard| Self::stream_key_for_shard(tenant_id, stream_base, shard))
            .collect()
    }

    fn stream_keys_for_tenant_ordered(
        tenant_id: &TenantId,
        stream_base: &str,
        start_shard: usize,
    ) -> Vec<String> {
        let shard_count =
            usize::try_from(SHARD_COUNT).expect("SHARD_COUNT must fit into usize for iteration");
        (0..shard_count)
            .map(|offset| {
                let shard = ((start_shard + offset) % shard_count) as u64;
                Self::stream_key_for_shard(tenant_id, stream_base, shard)
            })
            .collect()
    }

    fn decode_stream_entry(stream_key: &str, stream_id: &StreamId) -> Result<Delivery, EventError> {
        let payload = stream_id.get::<String>("data").ok_or_else(|| {
            EventError::Consumer(format!(
                "stream entry {} on {} missing data field",
                stream_id.id, stream_key
            ))
        })?;
        let event: EventEnvelope = serde_json::from_str(&payload)?;
        validate_stream_key(stream_key, &event.tenant_id)?;
        Ok(Delivery {
            delivery_id: stream_id.id.clone(),
            stream_key: stream_key.to_string(),
            event,
        })
    }

    async fn decode_stream_read(
        &self,
        consumer_group: &str,
        reply: StreamReadReply,
    ) -> Result<Vec<Delivery>, EventError> {
        let mut deliveries = Vec::new();
        for key in reply.keys {
            for stream_id in key.ids {
                match Self::decode_stream_entry(&key.key, &stream_id) {
                    Ok(delivery) => deliveries.push(delivery),
                    Err(e) if Self::is_tenant_isolation_error(&e) => {
                        // Metrics note: `kernel-data` currently lacks an established metrics facility (no prometheus crate).
                        // Instead of adding a new dependency just for this PR, we rely on tracing logs for
                        // observability of tenant isolation violations. A follow-up issue should add proper metrics
                        // (e.g., `kernel.event.tenant_isolation_violation_total`).
                        tracing::warn!(
                            stream_key = %key.key,
                            delivery_id = %stream_id.id,
                            error = %e,
                            "Tenant isolation violation detected while decoding stream entry; refusing to force-ack"
                        );
                        // The isolation-violating message is intentionally retained in the PEL for observability,
                        // but is intentionally NOT consumed (fail-closed security design).
                        // We continue to the next entry to avoid orphaning subsequent valid entries in the PEL.
                        continue;
                    }
                    Err(e) => {
                        tracing::error!(
                            stream_key = %key.key,
                            delivery_id = %stream_id.id,
                            error = %e,
                            "Poison pill detected: failed to decode stream entry. Force-acking to unblock consumer."
                        );
                        let mut conn = self.connection_manager.clone();
                        if let Err(ack_err) = conn
                            .xack::<_, _, _, i32>(&key.key, consumer_group, &[&stream_id.id])
                            .await
                        {
                            tracing::warn!(
                                stream_key = %key.key,
                                consumer_group = %consumer_group,
                                delivery_id = %stream_id.id,
                                error = %ack_err,
                                "Failed to force-ack poison pill; entry will be redelivered"
                            );
                            let err = EventError::Consumer(format!("failed to force-ack poison pill: {}", ack_err));
                            if deliveries.is_empty() {
                                return Err(err);
                            } else {
                                // Preserve earlier successfully decoded deliveries from this batch.
                                // Subsequent entries in this batch remain in the PEL but will be fetched on next poll.
                                return Ok(deliveries);
                            }
                        }
                        continue;
                    }
                }
            }
        }
        Ok(deliveries)
    }

    async fn decode_claimed(
        &self,
        stream_key: &str,
        consumer_group: &str,
        reply: StreamClaimReply,
    ) -> Result<Vec<Delivery>, EventError> {
        let mut deliveries = Vec::new();
        for stream_id in reply.ids {
            match Self::decode_stream_entry(stream_key, &stream_id) {
                Ok(delivery) => deliveries.push(delivery),
                Err(e) if Self::is_tenant_isolation_error(&e) => {
                    // Metrics note: `kernel-data` currently lacks an established metrics facility (no prometheus crate).
                    // Instead of adding a new dependency just for this PR, we rely on tracing logs for
                    // observability of tenant isolation violations. A follow-up issue should add proper metrics
                    // (e.g., `kernel.event.tenant_isolation_violation_total`).
                    tracing::warn!(
                        stream_key = %stream_key,
                        delivery_id = %stream_id.id,
                        error = %e,
                        "Tenant isolation violation detected while decoding claimed entry; refusing to force-ack"
                    );
                    // The isolation-violating message is intentionally retained in the PEL for observability,
                    // but is intentionally NOT consumed (fail-closed security design).
                    // We continue to the next entry to avoid orphaning subsequent valid entries in the PEL.
                    continue;
                }
                Err(e) => {
                    tracing::error!(
                        stream_key = %stream_key,
                        delivery_id = %stream_id.id,
                        error = %e,
                        "Poison pill detected in claimed entries: failed to decode. Force-acking to unblock consumer."
                    );
                    let mut conn = self.connection_manager.clone();
                    if let Err(ack_err) = conn
                        .xack::<_, _, _, i32>(stream_key, consumer_group, &[&stream_id.id])
                        .await
                    {
                        tracing::warn!(
                            stream_key = %stream_key,
                            consumer_group = %consumer_group,
                            delivery_id = %stream_id.id,
                            error = %ack_err,
                            "Failed to force-ack poison pill; entry will be redelivered"
                        );
                        let err = EventError::Consumer(format!("failed to force-ack poison pill: {}", ack_err));
                        if deliveries.is_empty() {
                            return Err(err);
                        } else {
                            // Preserve earlier successfully decoded deliveries from this batch.
                            // Subsequent entries in this batch remain in the PEL but will be fetched on next poll.
                            return Ok(deliveries);
                        }
                    }
                    continue;
                }
            }
        }
        Ok(deliveries)
    }

    fn build_read_options(
        consumer_group: &str,
        consumer_name: &str,
        max_count: usize,
        block_timeout: Option<Duration>,
        noack: bool,
    ) -> StreamReadOptions {
        let mut options = StreamReadOptions::default()
            .group(consumer_group, consumer_name)
            .count(max_count);
        if noack {
            options = options.noack();
        }
        match block_timeout {
            Some(timeout) => {
                let block_ms = usize::try_from(timeout.as_millis()).unwrap_or(usize::MAX);
                options.block(block_ms)
            }
            None => options,
        }
    }

    fn validate_retry_policy(policy: &RetryPolicy) -> Result<(), EventError> {
        match policy {
            RetryPolicy::Immediate => Ok(()),
            RetryPolicy::BackoffUntil(retry_at) => Err(EventError::Consumer(format!(
                "RetryPolicy::BackoffUntil({retry_at}) is not supported by RedisConsumer without a delayed retry queue",
            ))),
        }
    }

    fn handle_ack_result(
        acked: i32,
        stream_key: &str,
        delivery_id: &str,
    ) -> Result<(), EventError> {
        if acked == 0 {
            warn!(
                stream_key = stream_key,
                delivery_id = delivery_id,
                "xack reported no pending entry; treating ack as idempotent success"
            );
            return Ok(());
        }
        if acked < 0 {
            return Err(EventError::Consumer(format!(
                "failed to acknowledge stream entry {delivery_id} on {stream_key}: Redis reported a negative acknowledgement count"
            )));
        }
        Ok(())
    }

    fn is_busy_group_error(error: &RedisError) -> bool {
        error.code() == Some("BUSYGROUP")
    }

    fn is_nogroup_error(error: &RedisError) -> bool {
        error.code() == Some("NOGROUP")
    }

    fn is_tenant_isolation_error(error: &EventError) -> bool {
        matches!(error, EventError::TenantIsolation { .. })
    }

    fn consumer_group_cache_key(
        tenant_id: &TenantId,
        stream_base: &str,
        consumer_group: &str,
    ) -> String {
        // Use 'cache:cg' to avoid collision with stream key format '{tenant}:{base}:{shard}'
        format!("{tenant_id}:{stream_base}:cache:cg:{consumer_group}")
    }

    async fn ensure_consumer_groups(
        &self,
        tenant_id: &TenantId,
        stream_base: &str,
        consumer_group: &str,
        start_id: &str,
    ) -> Result<(), EventError> {
        for key in Self::stream_keys_for_tenant(tenant_id, stream_base) {
            let mut conn = self.connection_manager.clone();
            // Reliability Trade-off: Starting from "0" ensures at-least-once delivery by replaying
            // the full stream backlog. In a NOGROUP recovery scenario for an existing stream,
            // this can trigger a massive redelivery of historical events.
            // Using "$" would skip the backlog and only receive new messages.
            let create_group: Result<(), RedisError> = conn
                .xgroup_create_mkstream(&key, consumer_group, start_id)
                .await;
            if let Err(error) = create_group {
                if Self::is_busy_group_error(&error) {
                    continue;
                }
                return Err(EventError::Consumer(format!(
                    "failed to create consumer group for stream {key}: {error}"
                )));
            }
        }
        Ok(())
    }

    async fn ensure_consumer_groups_cached(
        &self,
        tenant_id: &TenantId,
        stream_base: &str,
        consumer_group: &str,
    ) -> Result<(), EventError> {
        let cache_key = Self::consumer_group_cache_key(tenant_id, stream_base, consumer_group);
        if self.ensured_consumer_groups.get(&cache_key).await.is_some() {
            return Ok(());
        }
        // Design note: Intentionally skipping a strict lock before calling ensure_consumer_groups
        // (which does network I/O) to avoid blocking other concurrent polls. The underlying
        // XGROUP CREATE operation is idempotent, so concurrent redundant calls are benign.
        self.ensure_consumer_groups(
            tenant_id,
            stream_base,
            consumer_group,
            &self.nogroup_recovery_start_id,
        )
        .await?;

        self.ensured_consumer_groups.insert(cache_key, ()).await;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn recover_nogroup_and_retry<T, F, Fut>(
        &self,
        tenant_id: &TenantId,
        stream_base: &str,
        stream_key: &str,
        consumer_group: &str,
        operation_name: &str,
        reply: Result<T, RedisError>,
        retry_op: F,
    ) -> Result<Result<T, RedisError>, EventError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, RedisError>>,
    {
        if let Err(error) = &reply {
            // Redis returns NOGROUP when the consumer group does not exist.
            if Self::is_nogroup_error(error) {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    stream_key = %stream_key,
                    consumer_group = %consumer_group,
                    "NOGROUP error detected during {}; evicting cache and recreating consumer group",
                    operation_name
                );
                let cache_key =
                    Self::consumer_group_cache_key(tenant_id, stream_base, consumer_group);
                self.ensured_consumer_groups.invalidate(&cache_key).await;
                self.ensure_consumer_groups_cached(tenant_id, stream_base, consumer_group)
                    .await?;

                return Ok(retry_op().await);
            }
        }
        Ok(reply)
    }

    async fn poll_once(
        &self,
        tenant_id: &TenantId,
        stream_base: &str,
        consumer_group: &str,
        consumer_name: &str,
        max_count: usize,
    ) -> Result<Vec<Delivery>, EventError> {
        let shard_count =
            usize::try_from(SHARD_COUNT).expect("SHARD_COUNT must fit into usize for iteration");
        let start_shard = self
            .next_poll_start_shard
            .fetch_add(1, AtomicOrdering::Relaxed)
            % shard_count;
        // In the rare event of usize overflow, the modulo produces a negligible fairness blip.
        let keys = Self::stream_keys_for_tenant_ordered(tenant_id, stream_base, start_shard);
        let mut deliveries = Vec::new();

        // Phase 1: Non-blocking scan across shards. We intentionally use per-shard `xread_options`
        // rather than Lua consolidation to avoid Lua server blocking, preserve per-shard NOGROUP
        // recovery semantics, and avoid partial Lua-script PEL orphaning where earlier read entries
        // are discarded if a later shard throws an error.
        for key in &keys {
            if deliveries.len() >= max_count {
                return Ok(deliveries);
            }

            let remaining = max_count - deliveries.len();
            let read_options = Self::build_read_options(
                consumer_group,
                consumer_name,
                remaining,
                None,
                false,
            );
            let key_strs = [key.as_str()];
            let id_strs = [">"];

            let mut conn = self.connection_manager.clone();
            let reply: Result<StreamReadReply, RedisError> =
                conn.xread_options(&key_strs, &id_strs, &read_options).await;

            let reply = self
                .recover_nogroup_and_retry(
                    tenant_id,
                    stream_base,
                    key,
                    consumer_group,
                    "poll_once",
                    reply,
                    || async {
                        let mut retry_conn = self.connection_manager.clone();
                        retry_conn
                            .xread_options(&key_strs, &id_strs, &read_options)
                            .await
                    },
                )
                .await;

            let reply = match reply {
                Ok(Ok(reply)) => reply,
                Ok(Err(e)) => {
                    if deliveries.is_empty() {
                        return Err(EventError::Consumer(format!(
                            "failed to read stream group entries: {e}"
                        )));
                    }
                    tracing::warn!(
                        stream_key = %key,
                        error = %e,
                        delivered_count = deliveries.len(),
                        "Stopping shard scan after partial deliveries because a later shard read failed"
                    );
                    break;
                }
                Err(e) => {
                    if deliveries.is_empty() {
                        return Err(e);
                    }
                    tracing::warn!(
                        stream_key = %key,
                        error = %e,
                        delivered_count = deliveries.len(),
                        "Stopping shard scan after partial deliveries because NOGROUP recovery failed"
                    );
                    break;
                }
            };
            match self.decode_stream_read(consumer_group, reply).await {
                Ok(mut shard_deliveries) => {
                    deliveries.append(&mut shard_deliveries);
                }
                Err(e) => {
                    if deliveries.is_empty() {
                        return Err(e);
                    }
                    tracing::warn!(
                        stream_key = %key,
                        error = %e,
                        delivered_count = deliveries.len(),
                        "Stopping shard scan after partial deliveries because a later shard decode failed"
                    );
                    break;
                }
            }
        }

        if !deliveries.is_empty() {
            return Ok(deliveries);
        }

        // Phase 2: Blocking fallback. Read at most one entry from at most max_count shards so
        // Redis never moves more messages into the PEL than this poll can return.
        let blocking_options = Self::build_read_options(
            consumer_group,
            consumer_name,
            1,
            Some(self.block_timeout),
            false,
        );

        let blocking_key_count = max_count.min(keys.len());
        let key_strs: Vec<&str> = keys
            .iter()
            .take(blocking_key_count)
            .map(|s| s.as_str())
            .collect();
        let id_strs: Vec<&str> = vec![">"; key_strs.len()];

        let mut conn = self.connection_manager.clone();
        let reply: Result<StreamReadReply, RedisError> = conn
            .xread_options(&key_strs, &id_strs, &blocking_options)
            .await;

        // We pass a synthetic stream_key (`*`) to `recover_nogroup_and_retry` for logging
        // purposes, as Phase 2 uses a single `XREAD` over multiple shard keys. This ensures
        // the NOGROUP recovery logs a logical aggregate stream rather than a misleading single shard.
        let stream_key_log = format!("{tenant_id}:{stream_base}:*");
        let reply = self
            .recover_nogroup_and_retry(
                tenant_id,
                stream_base,
                &stream_key_log,
                consumer_group,
                "blocking poll_once",
                reply,
                || async {
                    let mut retry_conn = self.connection_manager.clone();
                    retry_conn
                        .xread_options(&key_strs, &id_strs, &blocking_options)
                        .await
                },
            )
            .await?;

        let reply = reply.map_err(|e| {
            EventError::Consumer(format!("failed to read stream group entries: {e}"))
        })?;

        if reply.keys.is_empty() {
            return Ok(Vec::new());
        }

        self.decode_stream_read(consumer_group, reply).await
    }

    async fn claim_pending_once(
        &self,
        tenant_id: &TenantId,
        stream_base: &str,
        consumer_group: &str,
        consumer_name: &str,
        min_idle_ms: u64,
        max_count: usize,
    ) -> Result<Vec<Delivery>, EventError> {
        let shard_count =
            usize::try_from(SHARD_COUNT).expect("SHARD_COUNT must fit into usize for iteration");
        let start_shard = self
            .next_poll_start_shard
            .fetch_add(1, AtomicOrdering::Relaxed)
            % shard_count;
        let keys = Self::stream_keys_for_tenant_ordered(tenant_id, stream_base, start_shard);
        let mut claimed = Vec::new();

        for key in keys {
            if claimed.len() >= max_count {
                break;
            }

            let mut next_stream_id = "0-0".to_string();

            // Limit loop iterations to prevent an infinite empty scan (DoS loop) when
            // the pending entries list (PEL) is filled with messages that are not yet idle enough.
            const MAX_AUTOCLAIM_SCANS: usize = 10;
            let mut scan_count = 0;

            loop {
                if claimed.len() >= max_count {
                    break;
                }
                if scan_count >= MAX_AUTOCLAIM_SCANS {
                    tracing::warn!(
                        tenant_id = %tenant_id,
                        stream_key = %key,
                        consumer_group = %consumer_group,
                        scan_count = scan_count,
                        max_autoclaim_scans = MAX_AUTOCLAIM_SCANS,
                        claimed_count = claimed.len(),
                        max_count = max_count,
                        "MAX_AUTOCLAIM_SCANS exhausted before reaching max_count; stopping autoclaim scan for this shard to prevent DoS loop"
                    );
                    break;
                }
                scan_count += 1;

                let remaining = max_count - claimed.len();
                let mut conn = self.connection_manager.clone();
                let reply: Result<StreamAutoClaimReply, RedisError> = conn
                    .xautoclaim_options(
                        &key,
                        consumer_group,
                        consumer_name,
                        min_idle_ms,
                        &next_stream_id,
                        StreamAutoClaimOptions::default().count(remaining.min(100)),
                    )
                    .await;

                let reply = self
                    .recover_nogroup_and_retry(
                        tenant_id,
                        stream_base,
                        &key,
                        consumer_group,
                        "claim_pending_once",
                        reply,
                        || async {
                            let mut retry_conn = self.connection_manager.clone();
                            retry_conn
                                .xautoclaim_options(
                                    &key,
                                    consumer_group,
                                    consumer_name,
                                    min_idle_ms,
                                    &next_stream_id,
                                    StreamAutoClaimOptions::default().count(remaining.min(100)),
                                )
                                .await
                        },
                    )
                    .await;

                let reply = match reply {
                    Ok(Ok(reply)) => reply,
                    Ok(Err(e)) => {
                        if claimed.is_empty() {
                            return Err(EventError::Consumer(format!(
                                "failed to claim pending entries: {e}"
                            )));
                        }
                        tracing::warn!(
                            stream_key = %key,
                            error = %e,
                            claimed_count = claimed.len(),
                            "Stopping autoclaim scan after partial deliveries because a later shard read failed"
                        );
                        return Ok(claimed);
                    }
                    Err(e) => {
                        if claimed.is_empty() {
                            return Err(e);
                        }
                        tracing::warn!(
                            stream_key = %key,
                            error = %e,
                            claimed_count = claimed.len(),
                            "Stopping autoclaim scan after partial deliveries because NOGROUP recovery failed"
                        );
                        return Ok(claimed);
                    }
                };

                let claimed_entries = reply.claimed;
                let next_cursor = reply.next_stream_id;
                if claimed_entries.is_empty() {
                    if next_cursor == "0-0" {
                        break;
                    }
                    next_stream_id = next_cursor;
                    continue;
                }

                match self.decode_claimed(
                    &key,
                    consumer_group,
                    StreamClaimReply {
                        ids: claimed_entries,
                    },
                ).await {
                    Ok(mut dec_claimed) => claimed.append(&mut dec_claimed),
                    Err(e) => {
                        if claimed.is_empty() {
                            return Err(e);
                        }
                        tracing::warn!(
                            tenant_id = %tenant_id,
                            stream_key = %key,
                            consumer_group = %consumer_group,
                            claimed_count = claimed.len(),
                            error = %e,
                            "Stopping autoclaim scan after partial deliveries because a later shard decode failed"
                        );
                        // Preserve previously claimed messages rather than discarding them,
                        // returning early so the isolation error isn't silently swallowed.
                        return Ok(claimed);
                    }
                }

                if next_cursor == "0-0" {
                    break;
                }
                next_stream_id = next_cursor;
            }
        }

        if claimed.len() > max_count {
            tracing::warn!(
                tenant_id = %tenant_id,
                stream_base = %stream_base,
                claimed_count = claimed.len(),
                max_count = max_count,
                "XAUTOCLAIM fetched more messages than max_count; truncating result to max_count (excess messages remain in PEL)"
            );
            claimed.truncate(max_count);
        }

        Ok(claimed)
    }
}

#[async_trait]
impl ReliableConsumer for RedisConsumer {
    #[instrument(skip(self), fields(tenant_id = %tenant_id, stream_base = stream_base, consumer_group = consumer_group, consumer_name = consumer_name))]
    async fn poll(
        &self,
        tenant_id: &TenantId,
        stream_base: &str,
        consumer_group: &str,
        consumer_name: &str,
        max_count: usize,
    ) -> Result<Vec<Delivery>, EventError> {
        Self::validate_stream_base(stream_base)?;
        if max_count == 0 {
            return Ok(Vec::new());
        }

        self.ensure_consumer_groups_cached(tenant_id, stream_base, consumer_group)
            .await?;

        self.poll_once(
            tenant_id,
            stream_base,
            consumer_group,
            consumer_name,
            max_count,
        )
        .await
    }

    #[instrument(skip(self), fields(tenant_id = %tenant_id, stream_key = stream_key, consumer_group = consumer_group, delivery_id = delivery_id))]
    async fn ack(
        &self,
        tenant_id: &TenantId,
        stream_key: &str,
        consumer_group: &str,
        delivery_id: &str,
    ) -> Result<(), EventError> {
        validate_stream_key(stream_key, tenant_id)?;

        let mut conn = self.connection_manager.clone();
        let reply: Result<i32, RedisError> =
            conn.xack(stream_key, consumer_group, &[delivery_id]).await;

        match reply {
            Ok(acked) => Self::handle_ack_result(acked, stream_key, delivery_id),
            Err(e) if Self::is_nogroup_error(&e) => {
                tracing::warn!(
                    tenant_id = %tenant_id,
                    stream_key = stream_key,
                    consumer_group = consumer_group,
                    delivery_id = delivery_id,
                    "NOGROUP error during ack: consumer group does not exist"
                );
                Err(EventError::Consumer(format!(
                    "failed to acknowledge stream entry {} on {} for group {}: consumer group does not exist",
                    delivery_id, stream_key, consumer_group
                )))
            }
            Err(e) => Err(EventError::Consumer(format!(
                "failed to acknowledge stream entry: {e}"
            ))),
        }
    }

    /// NACKs a message.
    ///
    /// Note: Redis Streams does not support a native "requeue" without reordering.
    /// This implementation treats `RetryPolicy::Immediate` as a no-op, which keeps
    /// the message in the Pending Entries List (PEL). Subsequent calls to `claim_pending`
    /// (or this consumer's next poll) will eventually re-process the message.
    #[instrument(skip(self, policy), fields(tenant_id = %tenant_id, stream_key = %stream_key, consumer_group = %consumer_group))]
    async fn nack(
        &self,
        tenant_id: &TenantId,
        stream_key: &str,
        consumer_group: &str,
        delivery_id: &str,
        policy: RetryPolicy,
    ) -> Result<(), EventError> {
        validate_stream_key(stream_key, tenant_id)?;
        Self::validate_retry_policy(&policy)?;
        Ok(())
    }

    #[instrument(skip(self), fields(tenant_id = %tenant_id, stream_base = stream_base, consumer_group = consumer_group, consumer_name = consumer_name))]
    async fn claim_pending(
        &self,
        tenant_id: &TenantId,
        stream_base: &str,
        consumer_group: &str,
        consumer_name: &str,
        min_idle_ms: u64,
        max_count: usize,
    ) -> Result<Vec<Delivery>, EventError> {
        Self::validate_stream_base(stream_base)?;
        if max_count == 0 {
            return Ok(Vec::new());
        }

        self.ensure_consumer_groups_cached(tenant_id, stream_base, consumer_group)
            .await?;

        self.claim_pending_once(
            tenant_id,
            stream_base,
            consumer_group,
            consumer_name,
            min_idle_ms,
            max_count,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use redis::Value;
    use uuid::Uuid;

    fn sample_event() -> EventEnvelope {
        EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: TenantId::new("tenant-1").unwrap(),
            order_mode: crate::event::OrderMode::Entity {
                entity_id: Uuid::now_v7(),
                seq: Some(7),
            },
            payload: serde_json::json!({"ok": true}),
            created_at: Utc::now(),
            event_type: "sample.created".to_string(),
        }
    }

    #[test]
    fn test_calculate_shard_matches_producer_contract() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let event = sample_event();
        let shard = crate::event::calculate_shard(&event.order_mode.shard_input(&tenant_id));
        assert!(shard < SHARD_COUNT);
        assert_eq!(
            RedisConsumer::stream_key_for_shard(&tenant_id, "events", shard),
            format!("tenant-1:events:{shard}")
        );
    }

    #[test]
    fn test_validate_stream_base_rejects_invalid_names() {
        assert!(RedisConsumer::validate_stream_base("events").is_ok());
        assert!(RedisConsumer::validate_stream_base("").is_err());
        assert!(RedisConsumer::validate_stream_base("events:invalid").is_err());
    }

    #[test]
    fn test_validate_block_timeout_rejects_zero() {
        assert!(RedisConsumer::validate_block_timeout(Duration::from_millis(1)).is_ok());
        let err =
            RedisConsumer::validate_block_timeout(Duration::ZERO).expect_err("zero must fail");
        assert!(matches!(err, EventError::Consumer(_)));
    }

    #[test]
    fn test_validate_retry_policy_rejects_delayed_retry_without_queue() {
        let policy = RetryPolicy::BackoffUntil(Utc::now());
        let err = RedisConsumer::validate_retry_policy(&policy).expect_err("backoff must fail");
        assert!(matches!(err, EventError::Consumer(_)));
    }

    #[test]
    fn test_handle_ack_result_treats_zero_count_as_idempotent_success() {
        assert!(RedisConsumer::handle_ack_result(0, "tenant-1:events:4", "1-0").is_ok());
    }

    #[test]
    fn test_handle_ack_result_accepts_positive_counts() {
        assert!(RedisConsumer::handle_ack_result(1, "tenant-1:events:4", "1-0").is_ok());
    }

    #[test]
    fn test_validate_retry_policy_behavior() {
        // Immediate is treated as a supported no-op (leave in PEL)
        let ok = RedisConsumer::validate_retry_policy(&RetryPolicy::Immediate);
        assert!(ok.is_ok());

        // Others (delayed) are rejected because we have no delay queue logic
        let err =
            RedisConsumer::validate_retry_policy(&RetryPolicy::BackoffUntil(chrono::Utc::now()));
        assert!(err.is_err());
    }

    #[test]
    fn test_handle_ack_result_rejects_negative_counts() {
        let err = RedisConsumer::handle_ack_result(-1, "tenant-1:events:4", "1-0")
            .expect_err("negative ack count must fail");
        assert!(matches!(err, EventError::Consumer(_)));
    }

    #[test]
    fn test_decode_stream_entry_enforces_tenant_scope() {
        let event = sample_event();
        let payload = serde_json::to_string(&event).unwrap();
        let stream_id = StreamId {
            id: "1-0".to_string(),
            map: [("data".to_string(), Value::BulkString(payload.into_bytes()))]
                .into_iter()
                .collect(),
            milliseconds_elapsed_from_delivery: None,
            delivered_count: None,
        };

        let delivery =
            RedisConsumer::decode_stream_entry("tenant-1:events:4", &stream_id).expect("delivery");
        assert_eq!(delivery.stream_key, "tenant-1:events:4");
        assert_eq!(delivery.event.tenant_id, event.tenant_id);

        let err = RedisConsumer::decode_stream_entry("tenant-2:events:4", &stream_id)
            .expect_err("tenant mismatch must fail");
        assert!(matches!(err, EventError::TenantIsolation { .. }));
    }

    #[test]
    fn test_decode_stream_entry_payload_can_be_reused_for_retry() {
        let event = sample_event();
        let payload = serde_json::to_string(&event).unwrap();
        let stream_id = StreamId {
            id: "1-0".to_string(),
            map: [(
                "data".to_string(),
                Value::BulkString(payload.clone().into_bytes()),
            )]
            .into_iter()
            .collect(),
            milliseconds_elapsed_from_delivery: None,
            delivered_count: None,
        };

        let delivery =
            RedisConsumer::decode_stream_entry("tenant-1:events:4", &stream_id).expect("delivery");
        let encoded = serde_json::to_string(&delivery.event).expect("encode");
        assert_eq!(encoded, payload);
    }

    #[tokio::test]
    async fn test_decode_stream_read_fails_if_xack_fails() {
        let node = Redis::default()
            .with_tag("7.2-alpine")
            .start()
            .await
            .expect("start redis");
        let port = node.get_host_port_ipv4(REDIS_PORT).await.expect("get port");
        let redis_url = format!("redis://127.0.0.1:{port}/");
        let client = redis::Client::open(redis_url).unwrap();
        let consumer = RedisConsumer::new(client.clone()).await.unwrap();

        // Create a STRING key so XACK will fail with WRONGTYPE
        let mut conn = client.get_connection_manager().await.unwrap();
        let _: () = redis::cmd("SET")
            .arg("tenant-1:events:0")
            .arg("not a stream")
            .query_async(&mut conn)
            .await
            .unwrap();

        // Construct a StreamReadReply with a poison pill
        let stream_id = StreamId {
            id: "1-0".to_string(),
            map: [("data".to_string(), redis::Value::BulkString(b"invalid_json".to_vec()))]
                .into_iter()
                .collect(),
            milliseconds_elapsed_from_delivery: None,
            delivered_count: None,
        };
        let reply = redis::streams::StreamReadReply {
            keys: vec![redis::streams::StreamKey {
                key: "tenant-1:events:0".to_string(),
                ids: vec![stream_id],
            }],
        };

        let result = consumer.decode_stream_read("mygroup", reply).await;

        // Since `deliveries` is empty (this is the first entry), XACK failure should return Err
        // to strictly fail-closed rather than swallowing the Redis health anomaly.
        assert!(result.is_err(), "must return Err when xack fails on an empty batch");
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("failed to force-ack poison pill"));
    }

    #[tokio::test]
    async fn test_claim_pending_once_round_robin() -> Result<(), EventError> {
        let node = Redis::default()
            .with_tag("7.2-alpine")
            .start()
            .await
            .expect("start redis");
        let port = node.get_host_port_ipv4(REDIS_PORT).await.expect("get port");
        let redis_url = format!("redis://127.0.0.1:{port}/");
        let client = redis::Client::open(redis_url)
            .map_err(|e| EventError::Consumer(format!("failed to open redis client: {e}")))?;
        let consumer = RedisConsumer::new(client.clone()).await?;

        let tenant_id = TenantId::new("tenant-round-robin").unwrap();
        let stream_base = "test_round_robin_stream";
        let consumer_group = "test_group";
        let consumer_name = "test_consumer";

        // Teardown before setup
        let mut conn = client.get_connection_manager().await.unwrap();
        for key in RedisConsumer::stream_keys_for_tenant(&tenant_id, stream_base) {
            let _: () = redis::cmd("DEL")
                .arg(&key)
                .query_async(&mut conn)
                .await
                .unwrap();
        }

        consumer
            .ensure_consumer_groups_cached(&tenant_id, stream_base, consumer_group)
            .await?;

        let keys = RedisConsumer::stream_keys_for_tenant_ordered(&tenant_id, stream_base, 0);
        let key1 = &keys[0];
        let key2 = &keys[1];

        // Seed pending entries in at least two different shards.
        let mut event_a = sample_event();
        event_a.tenant_id = tenant_id.clone();
        let payload_a = serde_json::to_string(&event_a).unwrap();
        let _: () = redis::cmd("XADD")
            .arg(key1)
            .arg("*")
            .arg("data")
            .arg(&payload_a)
            .query_async(&mut conn)
            .await
            .unwrap();

        let mut event_b = sample_event();
        event_b.tenant_id = tenant_id.clone();
        let payload_b = serde_json::to_string(&event_b).unwrap();
        let _: () = redis::cmd("XADD")
            .arg(key2)
            .arg("*")
            .arg("data")
            .arg(&payload_b)
            .query_async(&mut conn)
            .await
            .unwrap();

        // Have "other_consumer" read them to put them in PEL
        let _: () = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(consumer_group)
            .arg("other_consumer")
            .arg("STREAMS")
            .arg(key1)
            .arg(key2)
            .arg(">")
            .arg(">")
            .query_async(&mut conn)
            .await
            .unwrap();

        let mut seen_streams = std::collections::HashSet::new();
        // Since it loops through SHARD_COUNT, we try just enough times to cover both shards.
        for _ in 0..(SHARD_COUNT + 5) {
            let res = consumer
                .claim_pending_once(&tenant_id, stream_base, consumer_group, consumer_name, 0, 1)
                .await?;
            for delivery in res {
                seen_streams.insert(delivery.stream_key);
            }
        }

        assert!(seen_streams.contains(key1.as_str()), "missing key1");
        assert!(seen_streams.contains(key2.as_str()), "missing key2");

        // Clean up
        for key in RedisConsumer::stream_keys_for_tenant(&tenant_id, stream_base) {
            let _: () = redis::cmd("DEL")
                .arg(&key)
                .query_async(&mut conn)
                .await
                .unwrap();
        }

        Ok(())
    }
}
