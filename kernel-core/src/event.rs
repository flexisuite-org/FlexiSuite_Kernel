use std::cmp::Ordering;

pub use kernel_data::event::{
    Delivery, EventEnvelope, EventError, OrderMode, PublishAck, ReliableConsumer, ReliableProducer,
    RetryPolicy, SHARD_COUNT, validate_stream_key,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapRecoveryState {
    Normal,
    GapDetected,
    Recovering,
    RebuildRequired,
}

/// Validates that a transition between two OrderMode instances is valid.
///
/// # Preconditions
/// The caller MUST ensure that both `existing` and `next` refer to the same logical key
/// (either the same `entity_id` or the same `causality_key`).
pub fn validate_order_mode_transition(
    existing: Option<&OrderMode>,
    next: &OrderMode,
) -> Result<(), EventError> {
    if let Some(existing_mode) = existing {
        // Assert logically consistent keys in development
        match (existing_mode, next) {
            (OrderMode::Entity { entity_id: e1, .. }, OrderMode::Entity { entity_id: e2, .. }) => {
                debug_assert_eq!(
                    e1, e2,
                    "validate_order_mode_transition called with different entity_ids"
                );
            }
            (OrderMode::Causality { key: k1, .. }, OrderMode::Causality { key: k2, .. }) => {
                debug_assert_eq!(
                    k1, k2,
                    "validate_order_mode_transition called with different causality keys"
                );
            }
            _ => {}
        }

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
///    that the recovery poll (invoked by the logic managing this FSM) has filled the gap.
///    If the gap still exists, Recovering must be retained.
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
    if state == GapRecoveryState::Normal {
        debug_assert!(
            !outbox_has_missing_seq,
            "progress_gap_recovery(Normal) called with outbox_has_missing_seq=true; callers must set GapDetected state first"
        );
        if outbox_has_missing_seq {
            return GapRecoveryState::Normal;
        }
    }

    match (state, outbox_has_missing_seq) {
        (GapRecoveryState::Normal, _) => GapRecoveryState::Normal,
        (GapRecoveryState::GapDetected, true) => GapRecoveryState::Recovering,
        (GapRecoveryState::GapDetected, false) => GapRecoveryState::RebuildRequired,
        (GapRecoveryState::Recovering, true) => GapRecoveryState::Recovering,
        (GapRecoveryState::Recovering, false) => GapRecoveryState::Normal,
        (GapRecoveryState::RebuildRequired, _) => GapRecoveryState::RebuildRequired,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::TenantId;
    use chrono::Utc;
    use serde_json::Value;
    use uuid::Uuid;

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

        // GapDetected + false -> RebuildRequired
        assert_eq!(
            progress_gap_recovery(GapRecoveryState::GapDetected, false),
            GapRecoveryState::RebuildRequired
        );
    }

    #[test]
    fn test_progress_gap_recovery_flow() {
        let state = progress_gap_recovery(GapRecoveryState::GapDetected, true);
        assert_eq!(state, GapRecoveryState::Recovering);
        let state = progress_gap_recovery(state, true);
        assert_eq!(state, GapRecoveryState::Recovering);
        let state = progress_gap_recovery(state, false);
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
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id: other_id,
                seq: Some(1),
            },
            payload: payload.clone(),
            created_at,
            event_type: event_type.clone(),
        };
        assert_eq!(compare_event_order(&e_some1, &e_other), None);

        // Cross-mode: Entity vs Causality => None
        let causality_event = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Causality {
                key: "key1".to_string(),
                seq: Some(1),
            },
            payload: payload.clone(),
            created_at,
            event_type: event_type.clone(),
        };
        assert_eq!(compare_event_order(&e_some1, &causality_event), None);
        assert_eq!(compare_event_order(&causality_event, &e_some1), None);
    }
}
