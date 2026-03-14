use crate::auth_context::TenantId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum EventError {
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("store error: {0}")]
    Store(String),
    #[error("producer error: {0}")]
    Producer(String),
    #[error("consumer error: {0}")]
    Consumer(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "order_mode", rename_all = "snake_case")]
pub enum OrderMode {
    Entity {
        entity_id: Uuid,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },
    Causality {
        key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "namespace", rename_all = "snake_case")]
pub enum OrderingKey {
    Entity { entity_id: Uuid },
    Causality { key: String },
}

impl OrderingKey {
    pub fn logical_key(&self) -> String {
        match self {
            OrderingKey::Entity { entity_id } => entity_id.to_string(),
            OrderingKey::Causality { key } => key.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantScopedOrderingKey {
    pub tenant_id: TenantId,
    #[serde(flatten)]
    pub ordering: OrderingKey,
}

impl TenantScopedOrderingKey {
    pub fn logical_key(&self) -> String {
        self.ordering.logical_key()
    }

    pub fn shard_input(&self) -> String {
        format!("{}:{}", self.tenant_id, self.logical_key())
    }
}

impl OrderMode {
    pub fn entity_or_causality_key(&self) -> String {
        self.ordering_key().logical_key()
    }

    pub fn ordering_key(&self) -> OrderingKey {
        match self {
            OrderMode::Entity { entity_id, .. } => OrderingKey::Entity {
                entity_id: *entity_id,
            },
            OrderMode::Causality { key, .. } => OrderingKey::Causality { key: key.clone() },
        }
    }

    pub fn tenant_scoped_ordering_key(&self, tenant_id: &TenantId) -> TenantScopedOrderingKey {
        TenantScopedOrderingKey {
            tenant_id: tenant_id.clone(),
            ordering: self.ordering_key(),
        }
    }

    pub fn shard_input(&self, tenant_id: &TenantId) -> String {
        self.tenant_scoped_ordering_key(tenant_id).shard_input()
    }

    pub fn seq(&self) -> Option<u64> {
        match self {
            OrderMode::Entity { seq, .. } => *seq,
            OrderMode::Causality { seq, .. } => *seq,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: Uuid,
    pub tenant_id: TenantId,
    pub order_mode: OrderMode,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
    pub event_type: String, // Added for routing/filtering
}

#[derive(Debug, Clone)]
pub struct PublishAck {
    pub message_id: String,
}

#[derive(Debug, Clone)]
pub struct Delivery {
    pub delivery_id: String, // Redis ID
    /// The actual shard key this message came from.
    /// MUST be tenant-scoped (e.g., `{tenant_id}:{stream_base}:{shard}`).
    pub stream_key: String,
    pub event: EventEnvelope,
}

pub const SHARD_COUNT: u64 = 64;

/// Validates that a stream_key matches the given tenant_id.
/// Returns Ok(()) if the key starts with "{tenant_id}:", Err(EventError) otherwise.
pub fn validate_stream_key(stream_key: &str, tenant_id: &TenantId) -> Result<(), EventError> {
    let prefix = format!("{}:", tenant_id);
    if !stream_key.starts_with(&prefix) {
        return Err(EventError::Consumer(format!(
            "stream_key '{}' does not match tenant_id '{}'",
            stream_key, tenant_id
        )));
    }
    Ok(())
}

#[async_trait]
pub trait ReliableProducer: Send + Sync {
    async fn publish(
        &self,
        stream_base: &str,
        event: EventEnvelope,
    ) -> Result<PublishAck, EventError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryPolicy {
    Immediate,
    BackoffUntil(DateTime<Utc>),
}

#[async_trait]
pub trait ReliableConsumer: Send + Sync {
    /// Polls for messages from the given base stream.
    ///
    /// `stream_base` is a logical stream name (e.g., `orders`) and MUST NOT include tenant prefix.
    /// Implementations MUST scope by `tenant_id` and internally fan-out across shards
    /// (e.g., `{tenant_id}:{stream}:0`..`{tenant_id}:{stream}:63`).
    async fn poll(
        &self,
        tenant_id: &TenantId,
        stream_base: &str,
        consumer_group: &str,
        consumer_name: &str,
        max_count: usize,
    ) -> Result<Vec<Delivery>, EventError>;

    /// Acknowledges a message.
    ///
    /// `stream_key` should be the specific shard key from `Delivery::stream_key`.
    /// `stream_key` MUST be tenant-scoped (e.g. starting with `tenant_id:`).
    /// Implementations MUST validate that `stream_key` matches `tenant_id` to enforce isolation.
    async fn ack(
        &self,
        tenant_id: &TenantId,
        stream_key: &str,
        consumer_group: &str,
        delivery_id: &str,
    ) -> Result<(), EventError>;

    /// Nack (negative acknowledgement) a message for retry.
    ///
    /// The `policy` parameter defines when the message should be retried.
    /// Implementations are expected to handle this based on the storage backend:
    /// - For Redis Streams: This might involve implementing a delay queue or
    ///   retaining the pending delivery and retrying in-process, as Redis Streams
    ///   doesn't natively support visibility timeouts per message while preserving
    ///   per-key ordering.
    /// - Callers can expect at-least-once delivery; however, ordering might
    ///   be impacted during retries depending on the implementation.
    ///
    /// `stream_key` should be the specific shard key from `Delivery::stream_key`.
    /// `stream_key` MUST be tenant-scoped (e.g. starting with `tenant_id:`).
    /// Implementations MUST validate that `stream_key` matches `tenant_id` to enforce isolation.
    async fn nack(
        &self,
        tenant_id: &TenantId,
        stream_key: &str,
        consumer_group: &str,
        delivery_id: &str,
        policy: RetryPolicy,
    ) -> Result<(), EventError>;

    /// Claims pending deliveries from a logical stream for a consumer in the same group.
    ///
    /// - `tenant_id`: tenant scope used to build physical stream keys.
    /// - `stream_base`: logical stream name (e.g., `orders`) without tenant prefix.
    /// - `consumer_group`: Redis Streams consumer group name to claim from.
    /// - `consumer_name`: target consumer name that will receive claimed deliveries.
    /// - `min_idle_ms`: minimum idle time (milliseconds) before a pending delivery is claimable.
    /// - `max_count`: upper bound of deliveries to claim in one call.
    ///
    /// Returns claimed deliveries on success, or `EventError` on backend/network/protocol failure.
    /// Implementations should preserve at-least-once delivery semantics. Under concurrent claimers,
    /// ownership may shift across consumers after `min_idle_ms`, and callers must handle duplicates.
    async fn claim_pending(
        &self,
        tenant_id: &TenantId,
        stream_base: &str,
        consumer_group: &str,
        consumer_name: &str,
        min_idle_ms: u64,
        max_count: usize,
    ) -> Result<Vec<Delivery>, EventError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_stream_key() {
        let tenant_id = TenantId::new("tenant_a").expect("Valid tenant ID");

        // Valid case
        assert!(validate_stream_key("tenant_a:orders:0", &tenant_id).is_ok());

        // Invalid cases
        assert!(validate_stream_key("tenant_b:orders:0", &tenant_id).is_err());
        assert!(validate_stream_key("orders:0", &tenant_id).is_err());
        assert!(validate_stream_key("tenant_a_suffix:orders:0", &tenant_id).is_err());

        // Boundary cases: empty tenant prefix
        // Since validate_stream_key constructs prefix as "{tenant_id}:",
        // if tenant_id is "tenant_a", prefix is "tenant_a:".
        // ":orders:0" effectively has empty tenant part which won't match "tenant_a:"
        assert!(validate_stream_key(":orders:0", &tenant_id).is_err());

        // Special characters in TenantId
        // Allowed chars: alphanumeric, _, - (based on typical TenantId rules, assuming here)
        let special_tenant = TenantId::new("tenant-a_1").expect("Valid special tenant ID");
        assert!(validate_stream_key("tenant-a_1:orders:0", &special_tenant).is_ok());
        assert!(validate_stream_key("tenant-a:orders:0", &special_tenant).is_err());

        if let Ok(weird_tenant) = TenantId::new("tenant/a") {
            // If "tenant/a" is valid, then "tenant/a:..." should work
            assert!(validate_stream_key("tenant/a:stream:0", &weird_tenant).is_ok());
        }
    }

    #[test]
    fn test_tenant_scoped_ordering_key_preserves_namespace() {
        let tenant_id = TenantId::new("tenant_a").unwrap();
        let entity_id = Uuid::now_v7();
        let entity_key = OrderMode::Entity {
            entity_id,
            seq: Some(1),
        }
        .tenant_scoped_ordering_key(&tenant_id);
        let causality_key = OrderMode::Causality {
            key: entity_id.to_string(),
            seq: Some(1),
        }
        .tenant_scoped_ordering_key(&tenant_id);

        assert_ne!(entity_key, causality_key);
        assert_eq!(entity_key.logical_key(), entity_id.to_string());
        assert_eq!(causality_key.logical_key(), entity_id.to_string());
    }
}
