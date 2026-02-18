use async_trait::async_trait;
use kernel_core::event::{EventEnvelope, EventError, PublishAck, ReliableProducer, SHARD_COUNT};
use redis::aio::ConnectionManager;
use redis::Client;
use std::hash::Hasher;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, error, instrument};
use twox_hash::XxHash64;

#[derive(Clone)]
pub struct RedisProducer {
    connection_manager: ConnectionManager,
    publish_timeout: Duration,
    stream_maxlen: usize,
}

impl RedisProducer {
    const DEFAULT_PUBLISH_TIMEOUT: Duration = Duration::from_secs(5);
    // Stream size cap per shard. Uses Redis approximate MAXLEN trim for O(1)-ish write path.
    const DEFAULT_STREAM_MAXLEN: usize = 10_000;

    pub async fn new(client: Client) -> Result<Self, EventError> {
        let connection_manager = client.get_connection_manager().await.map_err(|e| {
            EventError::Producer(format!("failed to create connection manager: {}", e))
        })?;
        Ok(Self {
            connection_manager,
            publish_timeout: Self::DEFAULT_PUBLISH_TIMEOUT,
            stream_maxlen: Self::DEFAULT_STREAM_MAXLEN,
        })
    }

    pub async fn new_with_config(
        client: Client,
        publish_timeout: Duration,
        stream_maxlen: usize,
    ) -> Result<Self, EventError> {
        let connection_manager = client.get_connection_manager().await.map_err(|e| {
            EventError::Producer(format!("failed to create connection manager: {}", e))
        })?;
        Ok(Self {
            connection_manager,
            publish_timeout,
            stream_maxlen,
        })
    }

    fn calculate_shard(key: &str) -> u64 {
        let mut hasher = XxHash64::default();
        hasher.write(key.as_bytes());
        let num = hasher.finish();
        num % SHARD_COUNT
    }
}

#[async_trait]
impl ReliableProducer for RedisProducer {
    #[instrument(skip(self, event), fields(event_id = %event.event_id, tenant_id = %event.tenant_id))]
    async fn publish(
        &self,
        stream_base: &str,
        event: EventEnvelope,
    ) -> Result<PublishAck, EventError> {
        let shard = Self::calculate_shard(&event.order_mode.shard_input(&event.tenant_id));
        // Ensure tenant isolation in stream namespace
        let stream_key = format!("{}:{}:{}", event.tenant_id, stream_base, shard);

        let payload_json = serde_json::to_string(&event).map_err(EventError::Serialization)?;

        // ConnectionManager handles reconnections and multiplexing.
        let mut conn = self.connection_manager.clone();

        // Use MAXLEN ~ N to bound stream growth without per-insert exact trimming cost.
        let mut cmd = redis::cmd("XADD");
        cmd.arg(&stream_key)
            .arg("MAXLEN")
            .arg("~")
            .arg(self.stream_maxlen)
            .arg("*")
            .arg("data")
            .arg(payload_json);

        let id: String = timeout(self.publish_timeout, cmd.query_async(&mut conn))
        .await
        .map_err(|_| EventError::Producer("redis publish timed out".to_string()))?
        .map_err(|e| {
            let err_msg = format!("failed to xadd: {}", e);
            error!("{}", err_msg);
            EventError::Producer(err_msg)
        })?;

        debug!(message_id = %id, stream_key = %stream_key, "successfully published event");

        Ok(PublishAck { message_id: id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_shard_deterministic() {
        let key1 = "test-key-1";
        let shard1 = RedisProducer::calculate_shard(key1);
        let shard1_again = RedisProducer::calculate_shard(key1);
        assert_eq!(shard1, shard1_again);

        let key2 = "test-key-2";
        let shard2 = RedisProducer::calculate_shard(key2);

        // Unlikely collision but possible, but we just check deterministic behavior primarily.
        // And range check.
        assert!(shard1 < SHARD_COUNT);
        assert!(shard2 < SHARD_COUNT);
    }

    #[test]
    fn test_calculate_shard_distribution() {
        // Simple check to ensure we are not mapping everything to 0
        let mut shards = std::collections::HashSet::new();
        for i in 0..100 {
            shards.insert(RedisProducer::calculate_shard(&format!("key-{}", i)));
        }
        assert!(shards.len() > 40);
    }
}
