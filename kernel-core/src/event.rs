use crate::auth::TenantId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
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
    /// The actual shard key this message came from.
    /// MUST be tenant-scoped (e.g., `{tenant_id}:{stream_base}:{shard}`).
    pub stream_key: String,
    pub event: EventEnvelope,
}

pub const SHARD_COUNT: u64 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapRecoveryState {
    Normal,
    GapDetected,
    Recovering,
    RebuildRequired,
}

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

pub fn validate_order_mode_transition(
    existing: Option<&OrderMode>,
    next: &OrderMode,
) -> Result<(), EventError> {
    if let Some(existing_mode) = existing {
        if std::mem::discriminant(existing_mode) != std::mem::discriminant(next) {
            return Err(EventError::Producer(
                "mixed order_mode forbidden for same logical key".to_string(),
            ));
        }
    }
    Ok(())
}

pub fn compare_event_order(a: &EventEnvelope, b: &EventEnvelope) -> Option<Ordering> {
    match (&a.order_mode, &b.order_mode) {
        (
            OrderMode::Entity {
                entity_id: aid,
                seq: Some(aseq),
            },
            OrderMode::Entity {
                entity_id: bid,
                seq: Some(bseq),
            },
        ) if aid == bid => Some(aseq.cmp(bseq)),
        (
            OrderMode::Causality {
                key: akey,
                seq: Some(aseq),
            },
            OrderMode::Causality {
                key: bkey,
                seq: Some(bseq),
            },
        ) if akey == bkey => Some(aseq.cmp(bseq)),
        _ => None,
    }
}

/// REQ-EVENT-GAP-001: Gap detection occurs via the outbox/consumer layer (e.g., Redis Streams)
/// when a non-contiguous sequence ID is observed.
/// REQ-EVENT-GAP-002: progress_gap_recovery drives the FSM to resolve detected gaps.
///
/// Lifecycle/Contract:
/// 1. GapDetected is emitted by the consumer logic when `delivery.seq > expected_seq`.
/// 2. The event loop invokes `progress_gap_recovery` once per activity cycle when in a non-Normal state.
///    Invoking from Normal when `outbox_has_missing_seq` is false is treated as a no-op.
/// 3. Recovering -> Normal transition ignores `outbox_has_missing_seq` under the assumption
///    that the recovery poll (invoked by the logic managing this FSM) has either filled the gap 
///    by fetching missing events or confirmed the gap cannot be recovered from the source.
/// 4. RebuildRequired is an absorbing state indicating manual intervention or full re-sync is needed.
///    This occurs if a Gap is detected but the recovery source is empty or inaccessible.
///
/// Example:
/// ```
/// use kernel_core::event::{GapRecoveryState, progress_gap_recovery};
/// let mut state = GapRecoveryState::Normal;
/// let gap_detected = true;
/// if gap_detected {
///     state = GapRecoveryState::GapDetected;
/// }
/// let has_missing = true;
/// // ... later in event loop ...
/// state = progress_gap_recovery(state, has_missing);
/// ```
pub fn progress_gap_recovery(
    state: GapRecoveryState,
    outbox_has_missing_seq: bool,
) -> GapRecoveryState {
    match (state, outbox_has_missing_seq) {
        (GapRecoveryState::Normal, true) => GapRecoveryState::Recovering,
        (GapRecoveryState::Normal, false) => GapRecoveryState::Normal,
        (GapRecoveryState::GapDetected, true) => GapRecoveryState::Recovering,
        (GapRecoveryState::GapDetected, false) => GapRecoveryState::RebuildRequired,
        (GapRecoveryState::Recovering, _) => GapRecoveryState::Normal,
        (GapRecoveryState::RebuildRequired, _) => GapRecoveryState::RebuildRequired,
    }
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
    ///   re-inserting the message with a delay, as Redis Streams doesn't natively
    ///   support visibility timeouts per message.
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

        // Invalid TenantId creation (if TenantId::new validates)
        // Assuming TenantId::new rejects some chars, we test that implicit contract.
        // If TenantId allows anything, we skip this specific rejection test or adapt it.
        // Let's assume typical restrictions. If TenantId allows everything, this might fail,
        // so we'll just check if it fails to create or if it works, validate_stream_key handles it.
        if let Ok(weird_tenant) = TenantId::new("tenant/a") {
            // If "tenant/a" is valid, then "tenant/a:..." should work
            assert!(validate_stream_key("tenant/a:stream:0", &weird_tenant).is_ok());
        } else {
            // If it failed to create, that's also a passed "boundary test" for TenantId
            // but validate_stream_key test can't run on it.
        }
    }

    #[test]
    fn test_validate_order_mode_transition_rejects_mix() {
        let entity_id = Uuid::now_v7();
        let existing = OrderMode::Entity {
            entity_id,
            seq: Some(1),
        };
        let next = OrderMode::Causality {
            key: entity_id.to_string(),
            seq: Some(2),
        };
        assert!(validate_order_mode_transition(Some(&existing), &next).is_err());
    }

    #[test]
    fn test_validate_order_mode_transition_edge_cases() {
        let entity_id = Uuid::now_v7();
        let mode = OrderMode::Entity {
            entity_id,
            seq: Some(1),
        };

        // First event (None) -> Ok
        assert!(validate_order_mode_transition(None, &mode).is_ok());

        // Same kind -> Ok
        assert!(validate_order_mode_transition(Some(&mode), &mode).is_ok());

        let causality = OrderMode::Causality {
            key: "key".to_string(),
            seq: Some(1),
        };
        assert!(validate_order_mode_transition(Some(&causality), &causality).is_ok());
    }

    #[test]
    fn test_progress_gap_recovery_absorbing_states() {
        // Normal false -> Normal
        assert_eq!(
            progress_gap_recovery(GapRecoveryState::Normal, false),
            GapRecoveryState::Normal
        );

        // RebuildRequired is absorbing
        assert_eq!(
            progress_gap_recovery(GapRecoveryState::RebuildRequired, true),
            GapRecoveryState::RebuildRequired
        );
        assert_eq!(
            progress_gap_recovery(GapRecoveryState::RebuildRequired, false),
            GapRecoveryState::RebuildRequired
        );
    }

    #[test]
    fn test_progress_gap_recovery_flow() {
        let state = progress_gap_recovery(GapRecoveryState::GapDetected, true);
        assert_eq!(state, GapRecoveryState::Recovering);
        let state = progress_gap_recovery(state, true);
        assert_eq!(state, GapRecoveryState::Normal);
    }

    #[test]
    fn test_compare_event_order_edge_cases() {
        let entity_id = Uuid::now_v7();
        let tenant_id = TenantId::new("t1").unwrap();
        let payload = Value::Null;
        let created_at = Utc::now();
        let event_type = "test".to_string();

        let e_none = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: None,
            },
            payload: payload.clone(),
            created_at,
            event_type: event_type.clone(),
        };

        let e_some1 = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(1),
            },
            payload: payload.clone(),
            created_at,
            event_type: event_type.clone(),
        };

        let e_some2 = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(2),
            },
            payload: payload.clone(),
            created_at,
            event_type: event_type.clone(),
        };

        // Same entity both None => None (incomparable)
        assert_eq!(compare_event_order(&e_none, &e_none), None);

        // Same entity one None one Some => None
        assert_eq!(compare_event_order(&e_none, &e_some1), None);
        assert_eq!(compare_event_order(&e_some1, &e_none), None);

        // Same entity both Some => Some(Ordering)
        assert_eq!(
            compare_event_order(&e_some1, &e_some2),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_event_order(&e_some2, &e_some1),
            Some(Ordering::Greater)
        );

        // Different entity => None
        let other_id = Uuid::now_v7();
        let e_other = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id,
            order_mode: OrderMode::Entity {
                entity_id: other_id,
                seq: Some(1),
            },
            payload,
            created_at,
            event_type,
        };
        assert_eq!(compare_event_order(&e_some1, &e_other), None);
    }
}
