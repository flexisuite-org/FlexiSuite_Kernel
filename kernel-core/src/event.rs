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

impl OrderMode {
    pub fn key_string(&self) -> String {
        match self {
            OrderMode::Entity { entity_id, .. } => entity_id.to_string(),
            OrderMode::Causality { key, .. } => key.clone(),
        }
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
    pub order_mode: OrderMode,
    pub payload: Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PublishAck {
    pub message_id: String,
}

#[derive(Debug, Clone)]
pub struct Delivery {
    pub delivery_id: String, // Redis ID
    pub event: EventEnvelope,
}

#[async_trait]
pub trait ReliableProducer: Send + Sync {
    async fn publish(&self, stream: &str, event: EventEnvelope) -> Result<PublishAck, EventError>;
}

#[async_trait]
pub trait ReliableConsumer: Send + Sync {
    async fn poll(
        &self,
        stream: &str,
        consumer_group: &str,
        consumer_name: &str,
        max_count: usize,
    ) -> Result<Vec<Delivery>, EventError>;

    async fn ack(&self, stream: &str, consumer_group: &str, delivery_id: &str) -> Result<(), EventError>;

    async fn nack(
        &self,
        stream: &str,
        consumer_group: &str,
        delivery_id: &str,
        retry_at: DateTime<Utc>,
    ) -> Result<(), EventError>;

    async fn claim_pending(
        &self,
        stream: &str,
        consumer_group: &str,
        consumer_name: &str,
        min_idle_ms: u64,
        max_count: usize,
    ) -> Result<Vec<Delivery>, EventError>;
}
