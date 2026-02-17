use async_trait::async_trait;
use kernel_core::event::{EventEnvelope, EventError, OrderMode, PublishAck, ReliableProducer};
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Client};
use ring::digest::{Context, SHA256};
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, error, instrument};

const SHARD_COUNT: u64 = 64;

#[derive(Clone)]
pub struct RedisProducer {
    connection_manager: ConnectionManager,
}

impl RedisProducer {
    pub async fn new(client: Client) -> Result<Self, EventError> {
        let connection_manager = client.get_connection_manager().await.map_err(|e| {
            EventError::Producer(format!("failed to create connection manager: {}", e))
        })?;
        Ok(Self { connection_manager })
    }

    fn calculate_shard(key: &str) -> u64 {
        let mut context = Context::new(&SHA256);
        context.update(key.as_bytes());
        let digest = context.finish();
        let bytes = digest.as_ref();
        // Use first 8 bytes as u64 (Big Endian)
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&bytes[0..8]);
        let num = u64::from_be_bytes(buf);
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
        let key_str = match &event.order_mode {
            OrderMode::Entity { entity_id, .. } => {
                format!("{}:{}", event.tenant_id, entity_id)
            }
            OrderMode::Causality { key, .. } => {
                format!("{}:{}", event.tenant_id, key)
            }
        };

        let shard = Self::calculate_shard(&key_str);
        let stream_key = format!("{}:{}", stream_base, shard);

        let payload_json = serde_json::to_string(&event).map_err(EventError::Serialization)?;

        // ConnectionManager handles reconnections and multiplexing.
        let mut conn = self.connection_manager.clone();

        let id: String = timeout(
            Duration::from_secs(5),
            conn.xadd(&stream_key, "*", &[("data", payload_json)]),
        )
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
        assert!(shards.len() > 1);
    }
}
