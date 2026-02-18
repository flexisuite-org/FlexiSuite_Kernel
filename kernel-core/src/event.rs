use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;
use crate::auth::TenantId;

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

impl OrderMode {
    pub fn entity_or_causality_key(&self) -> String {
        match self {
            OrderMode::Entity { entity_id, .. } => entity_id.to_string(),
            OrderMode::Causality { key, .. } => key.clone(),
        }
    }

    pub fn shard_input(&self, tenant_id: &TenantId) -> String {
        format!("{}:{}", tenant_id, self.entity_or_causality_key())
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
    pub stream_key: String,  // The actual shard key this message came from
    pub event: EventEnvelope,
}

pub const SHARD_COUNT: u64 = 64;

#[async_trait]
pub trait ReliableProducer: Send + Sync {
    async fn publish(&self, stream_base: &str, event: EventEnvelope) -> Result<PublishAck, EventError>;
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
    /// `stream_key` is expected to be tenant-less logical shard identifier unless
    /// the backend contract documents otherwise; implementations MUST use `tenant_id`
    /// to enforce tenant isolation.
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
    ///   re-inserting the message with a delay, as Redis Streams doesn't natively
    ///   support visibility timeouts per message.
    /// - Callers can expect at-least-once delivery; however, ordering might
    ///   be impacted during retries depending on the implementation.
    ///
    /// `stream_key` should be the specific shard key from `Delivery::stream_key`.
    /// `stream_key` is expected to be tenant-less logical shard identifier unless
    /// the backend contract documents otherwise; implementations MUST use `tenant_id`
    /// to enforce tenant isolation.
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
