use kernel_core::event::{EventEnvelope, OrderMode, ReliableProducer, PublishAck, EventError};
use async_trait::async_trait;
use redis::{Client, AsyncCommands};
use redis::aio::ConnectionManager;
use ring::digest::{Context, SHA256};

#[derive(Clone)]
pub struct RedisProducer {
    connection_manager: ConnectionManager,
}

impl RedisProducer {
    pub async fn new(client: Client) -> Result<Self, EventError> {
        let connection_manager = client.get_connection_manager().await
            .map_err(|e| EventError::Producer(format!("failed to create connection manager: {}", e)))?;
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
        num % 64
    }
}

#[async_trait]
impl ReliableProducer for RedisProducer {
    async fn publish(&self, stream_base: &str, event: EventEnvelope) -> Result<PublishAck, EventError> {
        let key_str = match &event.order_mode {
            OrderMode::Entity { entity_id, .. } => entity_id.to_string(),
            OrderMode::Causality { key, .. } => key.clone(),
        };

        let shard = Self::calculate_shard(&key_str);
        let stream_key = format!("{}:{}", stream_base, shard);

        let payload_json = serde_json::to_string(&event)
            .map_err(EventError::Serialization)?;

        // ConnectionManager handles reconnections and multiplexing.
        let mut conn = self.connection_manager.clone();

        let id: String = conn.xadd(&stream_key, "*", &[("data", payload_json)])
            .await
            .map_err(|e| EventError::Producer(format!("failed to xadd: {}", e)))?;

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
        assert!(shard1 < 64);
        assert!(shard2 < 64);
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
