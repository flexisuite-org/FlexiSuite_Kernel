use std::cmp::Ordering;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use kernel_data::event::{
    Delivery, EventEnvelope, EventError, OrderMode, OrderingKey, PublishAck, ReliableConsumer,
    ReliableProducer, RetryPolicy, SHARD_COUNT, TenantScopedOrderingKey, calculate_shard,
    validate_stream_key,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GapRecoveryState {
    Normal,
    GapDetected,
    Recovering,
    RebuildRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapObservation {
    pub ordering_key: TenantScopedOrderingKey,
    pub expected_seq: u64,
    pub actual_seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryResolution {
    Apply,
    Duplicate,
    GapDetected(GapObservation),
    Deferred(GapObservation),
}

#[derive(Debug, Clone)]
pub enum GapRecoveryAction {
    AwaitingTimeout(GapObservation),
    ReplayApply {
        gap: GapObservation,
        event: Arc<EventEnvelope>,
    },
    MarkPoisonAndRebuild {
        ordering_key: TenantScopedOrderingKey,
    },
}

#[derive(Debug, Clone)]
pub struct GapTracker {
    ordering_key: TenantScopedOrderingKey,
    expected_seq: u64,
    state: GapRecoveryState,
    gap_started_at: Option<Instant>,
    active_gap: Option<GapObservation>,
}

impl GapTracker {
    pub fn new(ordering_key: TenantScopedOrderingKey, expected_seq: u64) -> Self {
        Self {
            ordering_key,
            expected_seq,
            state: GapRecoveryState::Normal,
            gap_started_at: None,
            active_gap: None,
        }
    }

    pub fn expected_seq(&self) -> u64 {
        self.expected_seq
    }

    pub fn ordering_key(&self) -> &TenantScopedOrderingKey {
        &self.ordering_key
    }

    pub fn state(&self) -> GapRecoveryState {
        self.state
    }

    pub fn observe_delivery(
        &mut self,
        delivery: &Delivery,
        now: Instant,
    ) -> Result<DeliveryResolution, EventError> {
        if self.state == GapRecoveryState::RebuildRequired {
            return Err(EventError::Consumer(format!(
                "delivery {} observed while rebuild is required for expected sequence {}",
                delivery.delivery_id, self.expected_seq
            )));
        }

        let delivery_seq = delivery.event.order_mode.seq().ok_or_else(|| {
            EventError::Consumer(format!(
                "delivery {} missing ordering sequence",
                delivery.delivery_id
            ))
        })?;
        let delivery_key = delivery
            .event
            .order_mode
            .tenant_scoped_ordering_key(&delivery.event.tenant_id);
        if delivery_key != self.ordering_key {
            return Err(EventError::Consumer(format!(
                "delivery {} ordering key mismatch: expected {:?}, got {:?}",
                delivery.delivery_id, self.ordering_key, delivery_key
            )));
        }

        match delivery_seq.cmp(&self.expected_seq) {
            Ordering::Less => Ok(DeliveryResolution::Duplicate),
            Ordering::Equal => {
                self.expected_seq = self.expected_seq.checked_add(1).ok_or_else(|| {
                    EventError::Consumer(format!(
                        "sequence overflow for ordering key {:?}",
                        self.ordering_key
                    ))
                })?;
                if let Some(active_gap) = self.active_gap.as_ref() {
                    if self.expected_seq < active_gap.actual_seq {
                        // Safety: active_gap is Some, which means a gap was previously detected
                        // and not yet closed. Therefore, self.state cannot logically be Normal here.
                        // We check the state explicitly to avoid debug_assert if internal state
                        // was manually mutated (e.g. in tests).
                        if self.state != GapRecoveryState::Normal {
                            self.state = progress_gap_recovery(self.state, true);
                        }
                        self.gap_started_at = Some(now);
                        self.active_gap = Some(GapObservation {
                            ordering_key: active_gap.ordering_key.clone(),
                            expected_seq: self.expected_seq,
                            actual_seq: active_gap.actual_seq,
                        });
                    } else {
                        self.state = GapRecoveryState::Normal;
                        self.gap_started_at = None;
                        self.active_gap = None;
                    }
                } else {
                    self.state = GapRecoveryState::Normal;
                    self.gap_started_at = None;
                    self.active_gap = None;
                }
                Ok(DeliveryResolution::Apply)
            }
            Ordering::Greater => {
                if let Some(active_gap) = self.active_gap.as_mut() {
                    // Update actual_seq to the maximum sequence seen so far across all out-of-order deliveries
                    // while this gap remains active. This tracks the highest 'discovered' bound of the gap.
                    if delivery_seq > active_gap.actual_seq {
                        active_gap.actual_seq = delivery_seq;
                    }

                    // Protocol: Subsequent out-of-order deliveries are deferred.
                    // Callers should buffer these and re-feed them to observe_delivery
                    // only after the current active gap is closed.
                    return Ok(DeliveryResolution::Deferred(active_gap.clone()));
                }
                let gap = GapObservation {
                    ordering_key: self.ordering_key.clone(),
                    expected_seq: self.expected_seq,
                    actual_seq: delivery_seq,
                };
                self.state = GapRecoveryState::GapDetected;
                self.gap_started_at.get_or_insert(now);
                self.active_gap = Some(gap.clone());
                Ok(DeliveryResolution::GapDetected(gap))
            }
        }
    }

    pub async fn recover_gap<F, Fut>(
        &mut self,
        now: Instant,
        gap_timeout: Duration,
        lookup_missing: F,
    ) -> Result<Option<GapRecoveryAction>, EventError>
    where
        F: FnOnce(GapObservation) -> Fut,
        Fut: Future<Output = Result<Option<EventEnvelope>, EventError>>,
    {
        let started_at = match self.gap_started_at {
            Some(started_at) => started_at,
            None => return Ok(None),
        };
        let gap = match self.active_gap.clone() {
            Some(gap) => gap,
            None => return Ok(None),
        };

        if now.duration_since(started_at) < gap_timeout {
            return Ok(Some(GapRecoveryAction::AwaitingTimeout(gap)));
        }

        let maybe_missing = lookup_missing(gap.clone()).await?;
        if let Some(event) = maybe_missing {
            let replay_key = event
                .order_mode
                .tenant_scoped_ordering_key(&event.tenant_id);
            let replay_seq = event.order_mode.seq();

            if replay_key == gap.ordering_key && replay_seq == Some(gap.expected_seq) {
                self.state = progress_gap_recovery(self.state, true);
                self.gap_started_at = Some(now); // リプレイ完了を待つためにタイムアウト基準をリセット
                // Note: The caller must handle the case where this event was already applied
                // through a natural observe_delivery (idempotency at apply site).
                return Ok(Some(GapRecoveryAction::ReplayApply {
                    gap,
                    event: Arc::new(event),
                }));
            } else {
                tracing::error!(
                    expected_key = ?gap.ordering_key,
                    got_key = ?replay_key,
                    expected_seq = gap.expected_seq,
                    got_seq = ?replay_seq,
                    event_id = %event.event_id,
                    "recover_gap: mismatched event returned from lookup_missing"
                );
                // Fall through to RebuildRequired with explicit error log above.
            }
        }

        self.state = GapRecoveryState::RebuildRequired;
        self.gap_started_at = None;
        self.active_gap = None;
        Ok(Some(GapRecoveryAction::MarkPoisonAndRebuild {
            ordering_key: gap.ordering_key,
        }))
    }

    pub fn confirm_gap_replay(
        &mut self,
        gap: &GapObservation,
        event: &EventEnvelope,
        now: Instant,
    ) -> Result<(), EventError> {
        let replay_key = event
            .order_mode
            .tenant_scoped_ordering_key(&event.tenant_id);
        if replay_key != self.ordering_key {
            return Err(EventError::Consumer(format!(
                "replay confirmation ordering key mismatch: expected {:?}, got {:?}",
                self.ordering_key, replay_key
            )));
        }
        if gap.ordering_key != self.ordering_key {
            return Err(EventError::Consumer(format!(
                "replay confirmation gap ordering key mismatch: expected {:?}, got {:?}",
                self.ordering_key, gap.ordering_key
            )));
        }

        let replay_seq = event.order_mode.seq().ok_or_else(|| {
            EventError::Consumer("replay confirmation missing ordering sequence".to_string())
        })?;
        if replay_seq != gap.expected_seq {
            return Err(EventError::Consumer(format!(
                "replay confirmation sequence mismatch: expected {}, got {}",
                gap.expected_seq, replay_seq
            )));
        }

        if replay_seq < self.expected_seq {
            // Sequence has already advanced (e.g., via recover_gap -> observe_delivery -> confirm_gap_replay).
            // Treat as idempotent success.
            return Ok(());
        }

        let active_gap = self.active_gap.as_ref().ok_or_else(|| {
            EventError::Consumer("no active gap to confirm replay against".to_string())
        })?;
        if active_gap.ordering_key != gap.ordering_key
            || active_gap.expected_seq != gap.expected_seq
        {
            return Err(EventError::Consumer(
                "replay confirmation does not match the active gap".to_string(),
            ));
        }

        self.expected_seq = self.expected_seq.checked_add(1).ok_or_else(|| {
            EventError::Consumer(format!(
                "sequence overflow during gap replay for ordering key {:?}",
                self.ordering_key
            ))
        })?;
        if self.expected_seq < active_gap.actual_seq {
            self.state = progress_gap_recovery(self.state, true);
            self.gap_started_at = Some(now);
            self.active_gap = Some(GapObservation {
                ordering_key: active_gap.ordering_key.clone(),
                expected_seq: self.expected_seq,
                actual_seq: active_gap.actual_seq,
            });
        } else {
            // Replay has filled the final missing sequence of the active gap.
            // We use GapRecoveryState::Recovering explicitly to guarantee transition to Normal,
            // safely bypassing the current state (e.g. GapDetected) which would incorrectly transition
            // to RebuildRequired if we mistakenly passed outbox_has_missing_seq=false.
            // This is intentional as the GapTracker's responsibility for the specific gap is complete.
            self.state = progress_gap_recovery(GapRecoveryState::Recovering, false);
            self.gap_started_at = None;
            self.active_gap = None;
        }
        Ok(())
    }
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
                if e1 != e2 {
                    return Err(EventError::Producer(format!(
                        "mismatched entity_id during order_mode transition: {} != {}",
                        e1, e2
                    )));
                }
            }
            (OrderMode::Causality { key: k1, .. }, OrderMode::Causality { key: k2, .. }) => {
                if k1 != k2 {
                    return Err(EventError::Producer(format!(
                        "mismatched causality key during order_mode transition: {} != {}",
                        k1, k2
                    )));
                }
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
    if state == GapRecoveryState::Normal && outbox_has_missing_seq {
        debug_assert!(
            false,
            "progress_gap_recovery(Normal) called with outbox_has_missing_seq=true; callers should set GapDetected state first"
        );
        return GapRecoveryState::GapDetected;
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
        let special_tenant = TenantId::new("tenant-a_1").expect("Valid special tenant ID");
        assert!(validate_stream_key("tenant-a_1:orders:0", &special_tenant).is_ok());
        assert!(validate_stream_key("tenant-a:orders:0", &special_tenant).is_err());

        // Deterministic rejection of invalid characters
        assert!(TenantId::new("tenant/a").is_err());
        assert!(
            validate_stream_key("tenant-a:stream:0", &TenantId::new("tenant-a").unwrap()).is_ok()
        );
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

    #[tokio::test]
    async fn test_gap_tracker_replays_missing_event_after_timeout() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let missing = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(3),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };
        let future = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id,
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(4),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };
        let delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: future,
        };

        let start = Instant::now();
        let mut tracker = GapTracker::new(
            delivery
                .event
                .order_mode
                .tenant_scoped_ordering_key(&delivery.event.tenant_id),
            3,
        );
        let observed = tracker.observe_delivery(&delivery, start).unwrap();
        assert!(matches!(observed, DeliveryResolution::GapDetected(_)));

        let action = tracker
            .recover_gap(
                start + Duration::from_secs(31),
                Duration::from_secs(30),
                |_| async { Ok(Some(missing)) },
            )
            .await
            .unwrap()
            .expect("recovery action");

        let (gap, replay) = match action {
            GapRecoveryAction::ReplayApply { gap, event } => (gap, event),
            other => panic!("unexpected recovery action: {other:?}"),
        };
        tracker
            .confirm_gap_replay(&gap, &replay, start + Duration::from_secs(31))
            .unwrap();
        let replayed = tracker.observe_delivery(&delivery, start + Duration::from_secs(32));
        assert!(matches!(replayed, Ok(DeliveryResolution::Apply)));
        assert_eq!(tracker.state(), GapRecoveryState::Normal);
    }

    #[tokio::test]
    async fn test_gap_tracker_marks_rebuild_when_missing_event_not_found() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let future = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id,
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(9),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };
        let delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: future,
        };

        let start = Instant::now();
        let mut tracker = GapTracker::new(
            delivery
                .event
                .order_mode
                .tenant_scoped_ordering_key(&delivery.event.tenant_id),
            7,
        );
        let observed = tracker.observe_delivery(&delivery, start).unwrap();
        assert!(matches!(observed, DeliveryResolution::GapDetected(_)));

        let action = tracker
            .recover_gap(
                start + Duration::from_secs(31),
                Duration::from_secs(30),
                |_| async { Ok(None) },
            )
            .await
            .unwrap()
            .expect("recovery action");

        assert!(matches!(
            action,
            GapRecoveryAction::MarkPoisonAndRebuild { ordering_key }
                if ordering_key.logical_key() == entity_id.to_string()
        ));
        assert_eq!(tracker.state(), GapRecoveryState::RebuildRequired);
    }

    #[tokio::test]
    async fn test_gap_tracker_keeps_rebuild_required_after_later_missing_sequence_is_absent() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let future = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(8),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };
        let replay_three = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(3),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };
        let delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: future,
        };

        let start = Instant::now();
        let mut tracker = GapTracker::new(
            delivery
                .event
                .order_mode
                .tenant_scoped_ordering_key(&delivery.event.tenant_id),
            3,
        );
        tracker.observe_delivery(&delivery, start).unwrap();

        let action = tracker
            .recover_gap(
                start + Duration::from_secs(31),
                Duration::from_secs(30),
                |_| async { Ok(Some(replay_three)) },
            )
            .await
            .unwrap()
            .expect("first recovery action");
        let (gap, replay) = match action {
            GapRecoveryAction::ReplayApply { gap, event } => (gap, event),
            other => panic!("unexpected recovery action: {other:?}"),
        };
        tracker
            .confirm_gap_replay(&gap, &replay, start + Duration::from_secs(31))
            .unwrap();

        let rebuild_action = tracker
            .recover_gap(
                start + Duration::from_secs(62),
                Duration::from_secs(30),
                |_| async { Ok::<Option<EventEnvelope>, EventError>(None) },
            )
            .await
            .unwrap()
            .expect("rebuild action");

        assert!(matches!(
            rebuild_action,
            GapRecoveryAction::MarkPoisonAndRebuild { ordering_key }
                if ordering_key.logical_key() == entity_id.to_string()
        ));
        assert_eq!(tracker.state(), GapRecoveryState::RebuildRequired);

        let err = tracker
            .observe_delivery(&delivery, start + Duration::from_secs(63))
            .expect_err("deliveries must be rejected after rebuild is required");
        assert!(matches!(err, EventError::Consumer(_)));
    }

    #[tokio::test]
    async fn test_gap_tracker_rejects_deliveries_while_rebuild_required() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let future = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(9),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };
        let delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: future.clone(),
        };

        let start = Instant::now();
        let mut tracker = GapTracker::new(
            delivery
                .event
                .order_mode
                .tenant_scoped_ordering_key(&delivery.event.tenant_id),
            7,
        );
        tracker.observe_delivery(&delivery, start).unwrap();
        tracker
            .recover_gap(
                start + Duration::from_secs(31),
                Duration::from_secs(30),
                |_| async { Ok::<Option<EventEnvelope>, EventError>(None) },
            )
            .await
            .unwrap()
            .expect("recovery action");

        let err = tracker
            .observe_delivery(
                &Delivery {
                    delivery_id: "2-0".to_string(),
                    stream_key: "tenant-1:events:0".to_string(),
                    event: EventEnvelope {
                        event_id: Uuid::now_v7(),
                        tenant_id,
                        order_mode: OrderMode::Entity {
                            entity_id,
                            seq: Some(7),
                        },
                        payload: Value::Null,
                        created_at: Utc::now(),
                        event_type: "entity.updated".to_string(),
                    },
                },
                start + Duration::from_secs(32),
            )
            .expect_err("deliveries must be rejected during rebuild");

        assert!(matches!(err, EventError::Consumer(_)));
        assert_eq!(tracker.state(), GapRecoveryState::RebuildRequired);
    }

    #[test]
    fn test_gap_tracker_keeps_first_gap_active_when_later_out_of_order_delivery_arrives() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let ordering_key = OrderMode::Entity {
            entity_id,
            seq: Some(1),
        }
        .tenant_scoped_ordering_key(&tenant_id);
        let mut tracker = GapTracker::new(ordering_key, 3);
        let first_delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: EventEnvelope {
                event_id: Uuid::now_v7(),
                tenant_id: tenant_id.clone(),
                order_mode: OrderMode::Entity {
                    entity_id,
                    seq: Some(8),
                },
                payload: Value::Null,
                created_at: Utc::now(),
                event_type: "entity.updated".to_string(),
            },
        };
        let second_delivery = Delivery {
            delivery_id: "2-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: EventEnvelope {
                event_id: Uuid::now_v7(),
                tenant_id,
                order_mode: OrderMode::Entity {
                    entity_id,
                    seq: Some(5),
                },
                payload: Value::Null,
                created_at: Utc::now(),
                event_type: "entity.updated".to_string(),
            },
        };

        let first_observed = tracker
            .observe_delivery(&first_delivery, Instant::now())
            .expect("first out-of-order delivery must open the gap");
        let active_gap = match first_observed {
            DeliveryResolution::GapDetected(gap) => gap,
            other => panic!("unexpected resolution: {other:?}"),
        };

        let deferred = tracker
            .observe_delivery(&second_delivery, Instant::now())
            .expect("subsequent out-of-order deliveries should be deferred");

        assert!(matches!(
            deferred,
            DeliveryResolution::Deferred(gap)
                if gap.expected_seq == active_gap.expected_seq && gap.actual_seq == active_gap.actual_seq
        ));
        let replay = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: first_delivery.event.tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(3),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };
        tracker
            .confirm_gap_replay(&active_gap, &replay, Instant::now())
            .expect("original gap must remain active for replay confirmation");
    }

    #[tokio::test]
    async fn test_gap_tracker_interleaved_observe_delivery_during_recovery() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let future = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(5),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };
        let delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: future,
        };

        let start = Instant::now();
        let mut tracker = GapTracker::new(
            delivery
                .event
                .order_mode
                .tenant_scoped_ordering_key(&delivery.event.tenant_id),
            3,
        );
        let observed = tracker.observe_delivery(&delivery, start).unwrap();
        assert!(matches!(observed, DeliveryResolution::GapDetected(_)));

        let missing = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(3),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };

        let action = tracker
            .recover_gap(
                start + Duration::from_secs(31),
                Duration::from_secs(30),
                |_| async { Ok(Some(missing)) },
            )
            .await
            .unwrap()
            .expect("recovery action");

        let (gap, replay) = match action {
            GapRecoveryAction::ReplayApply { gap, event } => (gap, event),
            other => panic!("unexpected recovery action: {other:?}"),
        };

        // Interleaved delivery of the exact missing sequence via `observe_delivery`.
        // This advances tracker.expected_seq from 3 to 4.
        let interleaved_delivery = Delivery {
            delivery_id: "2-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: (*replay).clone(),
        };
        let interleaved_res = tracker
            .observe_delivery(&interleaved_delivery, start + Duration::from_secs(32))
            .unwrap();
        assert_eq!(interleaved_res, DeliveryResolution::Apply);
        assert_eq!(tracker.expected_seq, 4);

        // Now confirm_gap_replay runs. Since replay.seq (3) < tracker.expected_seq (4),
        // it should idempotently return Ok(()).
        tracker
            .confirm_gap_replay(&gap, &replay, start + Duration::from_secs(33))
            .expect("confirm_gap_replay must succeed idempotently for interleaved sequence");

        assert_eq!(tracker.expected_seq, 4); // Expected sequence remains unchanged by duplicate confirmation
    }

    #[test]
    fn test_gap_tracker_allows_multiple_future_deliveries_while_gap_is_active() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let ordering_key = OrderMode::Entity {
            entity_id,
            seq: Some(1),
        }
        .tenant_scoped_ordering_key(&tenant_id);
        let mut tracker = GapTracker::new(ordering_key, 3);
        let first_delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: EventEnvelope {
                event_id: Uuid::now_v7(),
                tenant_id: tenant_id.clone(),
                order_mode: OrderMode::Entity {
                    entity_id,
                    seq: Some(8),
                },
                payload: Value::Null,
                created_at: Utc::now(),
                event_type: "entity.updated".to_string(),
            },
        };
        let second_delivery = Delivery {
            delivery_id: "2-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: EventEnvelope {
                event_id: Uuid::now_v7(),
                tenant_id,
                order_mode: OrderMode::Entity {
                    entity_id,
                    seq: Some(9),
                },
                payload: Value::Null,
                created_at: Utc::now(),
                event_type: "entity.updated".to_string(),
            },
        };

        let observed = tracker
            .observe_delivery(&first_delivery, Instant::now())
            .expect("first out-of-order delivery must open the gap");
        assert!(matches!(observed, DeliveryResolution::GapDetected(_)));

        let deferred = tracker
            .observe_delivery(&second_delivery, Instant::now())
            .expect("later out-of-order deliveries should remain deferred while recovering");
        assert!(matches!(
            deferred,
            DeliveryResolution::Deferred(gap) if gap.expected_seq == 3 && gap.actual_seq == 9
        ));
        assert_eq!(tracker.expected_seq(), 3);
        assert_eq!(tracker.state(), GapRecoveryState::GapDetected);
    }

    #[tokio::test]
    async fn test_gap_tracker_keeps_recovering_until_all_missing_sequences_replayed() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let future = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(8),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };
        let delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: future.clone(),
        };
        let replay_three = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(3),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };
        let replay_four = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id,
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(4),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };

        let start = Instant::now();
        let mut tracker = GapTracker::new(
            delivery
                .event
                .order_mode
                .tenant_scoped_ordering_key(&delivery.event.tenant_id),
            3,
        );
        let observed = tracker.observe_delivery(&delivery, start).unwrap();
        assert!(matches!(observed, DeliveryResolution::GapDetected(_)));

        let action = tracker
            .recover_gap(
                start + Duration::from_secs(31),
                Duration::from_secs(30),
                |_| async { Ok(Some(replay_three)) },
            )
            .await
            .unwrap()
            .expect("first recovery action");
        let (gap, replay) = match action {
            GapRecoveryAction::ReplayApply { gap, event } => (gap, event),
            other => panic!("unexpected recovery action: {other:?}"),
        };
        tracker
            .confirm_gap_replay(&gap, &replay, start + Duration::from_secs(31))
            .unwrap();

        assert_eq!(tracker.expected_seq(), 4);
        assert_eq!(tracker.state(), GapRecoveryState::Recovering);

        let next_action = tracker
            .recover_gap(
                start + Duration::from_secs(62),
                Duration::from_secs(30),
                |_| async { Ok(Some(replay_four)) },
            )
            .await
            .unwrap()
            .expect("second recovery action");

        let (next_gap, next_replay) = match next_action {
            GapRecoveryAction::ReplayApply { gap, event } => (gap, event),
            other => panic!("unexpected recovery action: {other:?}"),
        };
        assert_eq!(next_gap.expected_seq, 4);
        assert_eq!(next_gap.actual_seq, 8);
        tracker
            .confirm_gap_replay(&next_gap, &next_replay, start + Duration::from_secs(62))
            .unwrap();
        assert_eq!(tracker.expected_seq(), 5);
        assert_eq!(tracker.state(), GapRecoveryState::Recovering);
    }

    #[tokio::test]
    async fn test_gap_tracker_resets_timeout_for_next_missing_sequence_after_replay() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let future = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(8),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };
        let delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: future,
        };
        let replay_three = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id,
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(3),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };

        let start = Instant::now();
        let mut tracker = GapTracker::new(
            delivery
                .event
                .order_mode
                .tenant_scoped_ordering_key(&delivery.event.tenant_id),
            3,
        );
        let observed = tracker.observe_delivery(&delivery, start).unwrap();
        assert!(matches!(observed, DeliveryResolution::GapDetected(_)));

        let action = tracker
            .recover_gap(
                start + Duration::from_secs(31),
                Duration::from_secs(30),
                |_| async { Ok(Some(replay_three)) },
            )
            .await
            .unwrap()
            .expect("first recovery action");
        let (gap, replay) = match action {
            GapRecoveryAction::ReplayApply { gap, event } => (gap, event),
            other => panic!("unexpected recovery action: {other:?}"),
        };
        tracker
            .confirm_gap_replay(&gap, &replay, start + Duration::from_secs(31))
            .unwrap();

        let next_action = tracker
            .recover_gap(
                start + Duration::from_secs(59),
                Duration::from_secs(30),
                |_| async { Ok::<Option<EventEnvelope>, EventError>(None) },
            )
            .await
            .unwrap()
            .expect("awaiting timeout action");

        assert!(matches!(
            next_action,
            GapRecoveryAction::AwaitingTimeout(gap) if gap.expected_seq == 4 && gap.actual_seq == 8
        ));
        assert_eq!(tracker.state(), GapRecoveryState::Recovering);
    }

    #[tokio::test]
    async fn test_gap_tracker_keeps_active_gap_when_next_expected_delivery_arrives_naturally() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let future_delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: EventEnvelope {
                event_id: Uuid::now_v7(),
                tenant_id: tenant_id.clone(),
                order_mode: OrderMode::Entity {
                    entity_id,
                    seq: Some(8),
                },
                payload: Value::Null,
                created_at: Utc::now(),
                event_type: "entity.updated".to_string(),
            },
        };
        let next_expected_delivery = Delivery {
            delivery_id: "2-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: EventEnvelope {
                event_id: Uuid::now_v7(),
                tenant_id: tenant_id.clone(),
                order_mode: OrderMode::Entity {
                    entity_id,
                    seq: Some(4),
                },
                payload: Value::Null,
                created_at: Utc::now(),
                event_type: "entity.updated".to_string(),
            },
        };
        let replay_three = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(3),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };

        let start = Instant::now();
        let mut tracker = GapTracker::new(
            future_delivery
                .event
                .order_mode
                .tenant_scoped_ordering_key(&future_delivery.event.tenant_id),
            3,
        );
        tracker.observe_delivery(&future_delivery, start).unwrap();
        tracker.state = GapRecoveryState::Recovering;
        tracker.active_gap = Some(GapObservation {
            ordering_key: future_delivery
                .event
                .order_mode
                .tenant_scoped_ordering_key(&future_delivery.event.tenant_id),
            expected_seq: 4,
            actual_seq: 8,
        });
        tracker.expected_seq = 4;
        tracker.gap_started_at = Some(start);

        let resolution = tracker
            .observe_delivery(&next_expected_delivery, start + Duration::from_secs(1))
            .expect("next in-order delivery should apply");
        assert!(matches!(resolution, DeliveryResolution::Apply));
        assert_eq!(tracker.state(), GapRecoveryState::Recovering);
        assert_eq!(tracker.expected_seq(), 5);

        let awaited = tracker
            .recover_gap(
                start + Duration::from_secs(29),
                Duration::from_secs(30),
                |_| async { Ok::<Option<EventEnvelope>, EventError>(Some(replay_three)) },
            )
            .await
            .expect("gap recovery result")
            .expect("awaiting timeout action");
        assert!(matches!(
            awaited,
            GapRecoveryAction::AwaitingTimeout(gap) if gap.expected_seq == 5 && gap.actual_seq == 8
        ));
    }

    #[test]
    fn test_gap_tracker_rejects_foreign_ordering_key() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let tracked_entity_id = Uuid::now_v7();
        let other_entity_id = Uuid::now_v7();
        let mut tracker = GapTracker::new(
            OrderMode::Entity {
                entity_id: tracked_entity_id,
                seq: Some(1),
            }
            .tenant_scoped_ordering_key(&tenant_id),
            1,
        );
        let delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: EventEnvelope {
                event_id: Uuid::now_v7(),
                tenant_id,
                order_mode: OrderMode::Entity {
                    entity_id: other_entity_id,
                    seq: Some(1),
                },
                payload: Value::Null,
                created_at: Utc::now(),
                event_type: "entity.updated".to_string(),
            },
        };

        let err = tracker
            .observe_delivery(&delivery, Instant::now())
            .expect_err("foreign ordering key must be rejected");

        assert!(matches!(err, EventError::Consumer(_)));
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

    #[tokio::test]
    async fn test_gap_tracker_1_item_gap_immediate_replay() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let gap_seq = 3;
        let future_seq = 4;

        let future = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(future_seq),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };
        let delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: future,
        };

        let start = Instant::now();
        let mut tracker = GapTracker::new(
            delivery
                .event
                .order_mode
                .tenant_scoped_ordering_key(&delivery.event.tenant_id),
            gap_seq,
        );
        let observed = tracker.observe_delivery(&delivery, start).unwrap();
        assert!(matches!(observed, DeliveryResolution::GapDetected(_)));

        // Assert state is GapDetected
        assert_eq!(tracker.state(), GapRecoveryState::GapDetected);

        let replay = EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id,
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(gap_seq),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        };

        let gap = match tracker.active_gap.clone() {
            Some(g) => g,
            None => panic!("Gap should be active"),
        };

        // Confirm replay directly without going through recover_gap (which changes state to Recovering first)
        // This simulates immediate manual replay or tightly coupled recovery logic
        let res = tracker.confirm_gap_replay(&gap, &replay, start);
        assert!(res.is_ok());

        // Should transition to Normal, NOT RebuildRequired
        assert_eq!(tracker.state(), GapRecoveryState::Normal);
        assert_eq!(tracker.expected_seq, 4);
        assert!(tracker.active_gap.is_none());
    }

    #[test]
    fn test_gap_tracker_sequence_overflow() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let start = Instant::now();

        let mut tracker = GapTracker::new(
            TenantScopedOrderingKey {
                tenant_id: tenant_id.clone(),
                ordering: OrderingKey::Entity { entity_id },
            },
            u64::MAX,
        );

        let delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: EventEnvelope {
                event_id: Uuid::now_v7(),
                tenant_id: tenant_id.clone(),
                order_mode: OrderMode::Entity {
                    entity_id,
                    seq: Some(u64::MAX),
                },
                payload: Value::Null,
                created_at: Utc::now(),
                event_type: "test".to_string(),
            },
        };

        let res = tracker.observe_delivery(&delivery, start);
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("sequence overflow for ordering key")
        );
    }

    #[test]
    fn test_gap_tracker_observe_delivery_missing_sequence() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let start = Instant::now();
        let mut tracker = GapTracker::new(
            TenantScopedOrderingKey {
                tenant_id: tenant_id.clone(),
                ordering: OrderingKey::Entity { entity_id },
            },
            1,
        );

        let delivery = Delivery {
            delivery_id: "1-0".to_string(),
            stream_key: "tenant-1:events:0".to_string(),
            event: EventEnvelope {
                event_id: Uuid::now_v7(),
                tenant_id,
                order_mode: OrderMode::Entity {
                    entity_id,
                    seq: None, // Missing sequence
                },
                payload: Value::Null,
                created_at: Utc::now(),
                event_type: "test".to_string(),
            },
        };

        let res = tracker.observe_delivery(&delivery, start);
        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("missing ordering sequence")
        );
    }
    #[test]
    fn test_gap_tracker_refeeds_deferred_deliveries_after_gap_closure() {
        let tenant_id = TenantId::new("tenant-1").unwrap();
        let entity_id = Uuid::now_v7();
        let ordering_key = OrderMode::Entity {
            entity_id,
            seq: Some(1),
        }
        .tenant_scoped_ordering_key(&tenant_id);
        let mut tracker = GapTracker::new(ordering_key.clone(), 3);

        // 1. Out-of-order arrival (seq=5) -> GapDetected (expected 3)
        let delivery_5 = mock_delivery_entity(&tenant_id, entity_id, 5);
        let res = tracker
            .observe_delivery(&delivery_5, Instant::now())
            .unwrap();
        assert!(
            matches!(res, DeliveryResolution::GapDetected(gap) if gap.expected_seq == 3 && gap.actual_seq == 5)
        );

        // 2. Further out-of-order (seq=6) -> Deferred (actual_seq updated to 6)
        let delivery_6 = mock_delivery_entity(&tenant_id, entity_id, 6);
        let res = tracker
            .observe_delivery(&delivery_6, Instant::now())
            .unwrap();
        assert!(matches!(res, DeliveryResolution::Deferred(gap) if gap.actual_seq == 6));

        // 3. Replay arrival (seq=3) -> confirm_gap_replay (expected becomes 4)
        let replay_3 = delivery_3_mock(&tenant_id, entity_id, 3);
        let active_gap = tracker.active_gap.clone().unwrap();
        tracker
            .confirm_gap_replay(&active_gap, &replay_3, Instant::now())
            .unwrap();
        assert_eq!(tracker.expected_seq(), 4);

        // 4. Natural arrival (seq=4) -> Apply (expected becomes 5)
        let delivery_4 = mock_delivery_entity(&tenant_id, entity_id, 4);
        let res = tracker
            .observe_delivery(&delivery_4, Instant::now())
            .unwrap();
        assert!(matches!(res, DeliveryResolution::Apply));
        assert_eq!(tracker.expected_seq(), 5);

        // 5. Gap should be closed now because expected_seq (5) reached the first out-of-order point.
        // Re-feeding buffered delivery_5 -> Apply (expected becomes 6)
        let res = tracker
            .observe_delivery(&delivery_5, Instant::now())
            .unwrap();
        assert!(matches!(res, DeliveryResolution::Apply));
        assert_eq!(tracker.expected_seq(), 6);

        // 6. Re-feeding buffered delivery_6 -> Apply (expected becomes 7)
        let res = tracker
            .observe_delivery(&delivery_6, Instant::now())
            .unwrap();
        assert!(matches!(res, DeliveryResolution::Apply));
        assert_eq!(tracker.expected_seq(), 7);
        assert!(tracker.active_gap.is_none());
    }

    fn mock_delivery_entity(tenant_id: &TenantId, entity_id: Uuid, seq: u64) -> Delivery {
        Delivery {
            delivery_id: format!("{}-0", seq),
            stream_key: format!("{}:events:0", tenant_id),
            event: EventEnvelope {
                event_id: Uuid::now_v7(),
                tenant_id: tenant_id.clone(),
                order_mode: OrderMode::Entity {
                    entity_id,
                    seq: Some(seq),
                },
                payload: Value::Null,
                created_at: Utc::now(),
                event_type: "entity.updated".to_string(),
            },
        }
    }

    fn delivery_3_mock(tenant_id: &TenantId, entity_id: Uuid, seq: u64) -> EventEnvelope {
        EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(seq),
            },
            payload: Value::Null,
            created_at: Utc::now(),
            event_type: "entity.updated".to_string(),
        }
    }
}
