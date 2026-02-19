use crate::event::{EventEnvelope, EventError, PublishAck, ReliableProducer, SHARD_COUNT};
use async_trait::async_trait;
use redis::Client;
use redis::aio::ConnectionManager;
use std::hash::Hasher;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, error, instrument};
use twox_hash::XxHash64;

#[derive(Clone)]
pub struct RedisProducer {
    connection_manager: ConnectionManager,
    publish_timeout: Duration,
    // None means "no producer-side trimming" to preserve reliable delivery.
    stream_maxlen: Option<usize>,
}

impl RedisProducer {
    const DEFAULT_PUBLISH_TIMEOUT: Duration = Duration::from_secs(5);
    const MIN_STREAM_MAXLEN: usize = 100;

    pub async fn new(client: Client) -> Result<Self, EventError> {
        let connection_manager = client.get_connection_manager().await.map_err(|e| {
            EventError::Producer(format!("failed to create connection manager: {}", e))
        })?;
        Ok(Self {
            connection_manager,
            publish_timeout: Self::DEFAULT_PUBLISH_TIMEOUT,
            stream_maxlen: None,
        })
    }

    pub async fn new_with_config(
        client: Client,
        publish_timeout: Duration,
        stream_maxlen: Option<usize>,
    ) -> Result<Self, EventError> {
        Self::validate_stream_maxlen(stream_maxlen)?;

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

    fn validate_stream_base(stream_base: &str) -> Result<(), EventError> {
        if stream_base.is_empty() || stream_base.contains(':') {
            return Err(EventError::Producer(format!(
                "invalid stream_base: '{}'. Must not be empty or contain ':'",
                stream_base
            )));
        }
        Ok(())
    }

    fn validate_stream_maxlen(stream_maxlen: Option<usize>) -> Result<(), EventError> {
        if let Some(value) = stream_maxlen {
            if value < Self::MIN_STREAM_MAXLEN {
                return Err(EventError::Producer(format!(
                    "invalid stream_maxlen: {}. Must be >= {}",
                    value,
                    Self::MIN_STREAM_MAXLEN
                )));
            }
        }
        Ok(())
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
        Self::validate_stream_base(stream_base)?;

        let shard = Self::calculate_shard(&event.order_mode.shard_input(&event.tenant_id));
        // Ensure tenant isolation in stream namespace
        let stream_key = format!("{}:{}:{}", event.tenant_id, stream_base, shard);

        let payload_json = serde_json::to_string(&event).map_err(EventError::Serialization)?;

        // ConnectionManager handles reconnections and multiplexing.
        let mut conn = self.connection_manager.clone();

        // Reliable default: do not trim at publish time.
        // If configured, apply approximate MAXLEN trim.
        let mut cmd = redis::cmd("XADD");
        cmd.arg(&stream_key);
        if let Some(maxlen) = self.stream_maxlen {
            cmd.arg("MAXLEN").arg("~").arg(maxlen);
        }
        cmd.arg("*").arg("data").arg(payload_json);

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

    #[test]
    fn test_validate_stream_base() {
        // Valid cases
        assert!(RedisProducer::validate_stream_base("events").is_ok());
        assert!(RedisProducer::validate_stream_base("my-stream").is_ok());
        assert!(RedisProducer::validate_stream_base("valid_underscore").is_ok());

        // Invalid cases
        assert!(RedisProducer::validate_stream_base("").is_err());
        assert!(RedisProducer::validate_stream_base("invalid:colon").is_err());
        assert!(RedisProducer::validate_stream_base("colon:at:start").is_err());
        assert!(RedisProducer::validate_stream_base("end:").is_err());
    }

    #[test]
    fn test_validate_stream_maxlen() {
        assert!(RedisProducer::validate_stream_maxlen(None).is_ok());
        assert!(RedisProducer::validate_stream_maxlen(Some(1_000)).is_ok());
        assert!(RedisProducer::validate_stream_maxlen(Some(0)).is_err());
        assert!(RedisProducer::validate_stream_maxlen(Some(99)).is_err());
    }
}
