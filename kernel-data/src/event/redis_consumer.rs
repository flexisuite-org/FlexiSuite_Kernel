use crate::auth_context::TenantId;
use crate::event::{
    Delivery, EventEnvelope, EventError, ReliableConsumer, RetryPolicy, SHARD_COUNT,
    validate_stream_key,
};
use async_trait::async_trait;
use redis::aio::ConnectionManager;
use redis::streams::{
    StreamAutoClaimOptions, StreamAutoClaimReply, StreamClaimReply, StreamId, StreamReadOptions,
    StreamReadReply,
};
use redis::{AsyncCommands, Client, RedisError};
#[cfg(test)]
use std::hash::Hasher;
use std::time::Duration;
use tracing::instrument;
#[cfg(test)]
use twox_hash::XxHash64;

#[derive(Clone)]
pub struct RedisConsumer {
    connection_manager: ConnectionManager,
    block_timeout: Duration,
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
        })
    }

    pub async fn new_with_config(
        client: Client,
        block_timeout: Duration,
    ) -> Result<Self, EventError> {
        Self::validate_block_timeout(block_timeout)?;
        let connection_manager = client.get_connection_manager().await.map_err(|e| {
            EventError::Consumer(format!("failed to create connection manager: {e}"))
        })?;
        Ok(Self {
            connection_manager,
            block_timeout,
        })
    }

    #[cfg(test)]
    fn calculate_shard(key: &str) -> u64 {
        let mut hasher = XxHash64::default();
        hasher.write(key.as_bytes());
        hasher.finish() % SHARD_COUNT
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

    fn decode_stream_read(reply: StreamReadReply) -> Result<Vec<Delivery>, EventError> {
        let mut deliveries = Vec::new();
        for key in reply.keys {
            for stream_id in key.ids {
                deliveries.push(Self::decode_stream_entry(&key.key, &stream_id)?);
            }
        }
        Ok(deliveries)
    }

    fn decode_claimed(
        stream_key: &str,
        reply: StreamClaimReply,
    ) -> Result<Vec<Delivery>, EventError> {
        reply
            .ids
            .iter()
            .map(|stream_id| Self::decode_stream_entry(stream_key, stream_id))
            .collect()
    }

    fn validate_retry_policy(policy: &RetryPolicy) -> Result<(), EventError> {
        if let RetryPolicy::BackoffUntil(retry_at) = policy {
            return Err(EventError::Consumer(format!(
                "RetryPolicy::BackoffUntil({retry_at}) is not supported by RedisConsumer without a delayed retry queue"
            )));
        }
        Ok(())
    }

    fn handle_ack_result(
        acked: i32,
        stream_key: &str,
        delivery_id: &str,
    ) -> Result<(), EventError> {
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

    async fn ensure_consumer_groups(
        &self,
        tenant_id: &TenantId,
        stream_base: &str,
        consumer_group: &str,
    ) -> Result<(), EventError> {
        for key in Self::stream_keys_for_tenant(tenant_id, stream_base) {
            let mut conn = self.connection_manager.clone();
            let create_group: Result<(), RedisError> =
                conn.xgroup_create_mkstream(&key, consumer_group, "0").await;
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

    async fn poll_once(
        &self,
        tenant_id: &TenantId,
        stream_base: &str,
        consumer_group: &str,
        consumer_name: &str,
        max_count: usize,
    ) -> Result<Vec<Delivery>, EventError> {
        let keys = Self::stream_keys_for_tenant(tenant_id, stream_base);
        let ids = vec![">"; keys.len()];
        let options = StreamReadOptions::default()
            .group(consumer_group, consumer_name)
            .count(max_count)
            .block(self.block_timeout.as_millis() as usize);

        let mut conn = self.connection_manager.clone();
        let reply: StreamReadReply =
            conn.xread_options(&keys, &ids, &options)
                .await
                .map_err(|e| {
                    EventError::Consumer(format!("failed to read stream group entries: {e}"))
                })?;
        if reply.keys.is_empty() {
            return Ok(Vec::new());
        }
        Self::decode_stream_read(reply)
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
        let keys = Self::stream_keys_for_tenant(tenant_id, stream_base);
        let mut claimed = Vec::new();

        for key in keys {
            if claimed.len() >= max_count {
                break;
            }

            let mut next_stream_id = "0-0".to_string();
            loop {
                if claimed.len() >= max_count {
                    break;
                }

                let remaining = max_count - claimed.len();
                let options = StreamAutoClaimOptions::default().count(remaining.min(100));
                let mut conn = self.connection_manager.clone();
                let reply: StreamAutoClaimReply = conn
                    .xautoclaim_options(
                        &key,
                        consumer_group,
                        consumer_name,
                        min_idle_ms,
                        &next_stream_id,
                        options,
                    )
                    .await
                    .map_err(|e| {
                        EventError::Consumer(format!("failed to claim pending entries: {e}"))
                    })?;

                let claimed_entries = reply.claimed;
                let next_cursor = reply.next_stream_id;
                if claimed_entries.is_empty() {
                    if next_cursor == "0-0" {
                        break;
                    }
                    next_stream_id = next_cursor;
                    continue;
                }

                claimed.extend(Self::decode_claimed(
                    &key,
                    StreamClaimReply {
                        ids: claimed_entries,
                    },
                )?);

                if next_cursor == "0-0" {
                    break;
                }
                next_stream_id = next_cursor;
            }
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

        match self
            .poll_once(
                tenant_id,
                stream_base,
                consumer_group,
                consumer_name,
                max_count,
            )
            .await
        {
            Ok(deliveries) => Ok(deliveries),
            Err(EventError::Consumer(message)) if message.contains("NOGROUP") => {
                self.ensure_consumer_groups(tenant_id, stream_base, consumer_group)
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
            Err(error) => Err(error),
        }
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
        let acked: i32 = conn
            .xack(stream_key, consumer_group, &[delivery_id])
            .await
            .map_err(|e| {
                EventError::Consumer(format!("failed to acknowledge stream entry: {e}"))
            })?;
        Self::handle_ack_result(acked, stream_key, delivery_id)
    }

    #[instrument(skip(self, policy), fields(tenant_id = %tenant_id, stream_key = stream_key, consumer_group = consumer_group, delivery_id = delivery_id))]
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
        let _ = (consumer_group, delivery_id);

        match policy {
            RetryPolicy::Immediate => Err(EventError::Consumer(format!(
                "RetryPolicy::Immediate is not supported by RedisConsumer because requeueing would reorder stream entries on {stream_key}; retry the pending delivery in-process or reclaim it after idle timeout"
            ))),
            RetryPolicy::BackoffUntil(_) => unreachable!("validated above"),
        }
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

        match self
            .claim_pending_once(
                tenant_id,
                stream_base,
                consumer_group,
                consumer_name,
                min_idle_ms,
                max_count,
            )
            .await
        {
            Ok(deliveries) => Ok(deliveries),
            Err(EventError::Consumer(message)) if message.contains("NOGROUP") => {
                self.ensure_consumer_groups(tenant_id, stream_base, consumer_group)
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
            Err(error) => Err(error),
        }
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
        let shard = RedisConsumer::calculate_shard(&event.order_mode.shard_input(&tenant_id));
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
    fn test_handle_ack_result_rejects_zero_counts() {
        assert!(RedisConsumer::handle_ack_result(0, "tenant-1:events:4", "1-0").is_ok());
        assert!(RedisConsumer::handle_ack_result(1, "tenant-1:events:4", "1-0").is_ok());
    }

    #[test]
    fn test_validate_retry_policy_allows_immediate_retry() {
        let err = RedisConsumer::validate_retry_policy(&RetryPolicy::Immediate);
        assert!(err.is_ok());
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
        assert!(matches!(err, EventError::Consumer(_)));
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
}
