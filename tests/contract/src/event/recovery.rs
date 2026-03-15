use kernel_core::auth::TenantId;
use kernel_core::event::{
    Delivery, GapRecoveryAction, GapRecoveryState, GapTracker, OrderMode, progress_gap_recovery,
};
use serde_json::Value;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn entity_ordering_key(
    tenant_id: &TenantId,
    entity_id: Uuid,
) -> kernel_core::event::TenantScopedOrderingKey {
    OrderMode::Entity {
        entity_id,
        seq: None,
    }
    .tenant_scoped_ordering_key(tenant_id)
}

#[tokio::test]
async fn test_gap_recovery_found() {
    let state = progress_gap_recovery(GapRecoveryState::GapDetected, true);
    assert_eq!(state, GapRecoveryState::Recovering);
    let state = progress_gap_recovery(state, true);
    assert_eq!(state, GapRecoveryState::Recovering);
    let state = progress_gap_recovery(state, false);
    assert_eq!(state, GapRecoveryState::Normal);
}

#[tokio::test]
async fn test_gap_recovery_rebuild_required() {
    let state = progress_gap_recovery(GapRecoveryState::GapDetected, false);
    assert_eq!(state, GapRecoveryState::RebuildRequired);
}

#[tokio::test]
async fn test_gap_tracker_replays_after_timeout_when_outbox_has_missing_event() {
    let tenant_id = TenantId::new("tenant-1").unwrap();
    let entity_id = Uuid::now_v7();
    let missing = kernel_core::event::EventEnvelope {
        event_id: Uuid::now_v7(),
        tenant_id: tenant_id.clone(),
        order_mode: OrderMode::Entity {
            entity_id,
            seq: Some(3),
        },
        payload: Value::Null,
        created_at: chrono::Utc::now(),
        event_type: "entity.updated".to_string(),
    };
    let delivery = Delivery {
        delivery_id: "1-0".to_string(),
        stream_key: "tenant-1:events:0".to_string(),
        event: kernel_core::event::EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id,
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(4),
            },
            payload: Value::Null,
            created_at: chrono::Utc::now(),
            event_type: "entity.updated".to_string(),
        },
    };

    let start = Instant::now();
    let mut tracker = GapTracker::new(entity_ordering_key(&delivery.event.tenant_id, entity_id), 3);
    tracker.observe_delivery(&delivery, start).unwrap();

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

    let resolution = tracker
        .observe_delivery(&delivery, start + Duration::from_secs(32))
        .unwrap();
    assert!(matches!(
        resolution,
        kernel_core::event::DeliveryResolution::Apply
    ));
}

#[tokio::test]
async fn test_gap_tracker_resets_timeout_after_partial_replay() {
    let tenant_id = TenantId::new("tenant-1").unwrap();
    let entity_id = Uuid::now_v7();
    let delivery = Delivery {
        delivery_id: "1-0".to_string(),
        stream_key: "tenant-1:events:0".to_string(),
        event: kernel_core::event::EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(8),
            },
            payload: Value::Null,
            created_at: chrono::Utc::now(),
            event_type: "entity.updated".to_string(),
        },
    };
    let missing = kernel_core::event::EventEnvelope {
        event_id: Uuid::now_v7(),
        tenant_id,
        order_mode: OrderMode::Entity {
            entity_id,
            seq: Some(3),
        },
        payload: Value::Null,
        created_at: chrono::Utc::now(),
        event_type: "entity.updated".to_string(),
    };

    let start = Instant::now();
    let mut tracker = GapTracker::new(entity_ordering_key(&delivery.event.tenant_id, entity_id), 3);
    tracker.observe_delivery(&delivery, start).unwrap();

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

    let next_action = tracker
        .recover_gap(
            start + Duration::from_secs(59),
            Duration::from_secs(30),
            |_| async { Ok::<Option<kernel_core::event::EventEnvelope>, _>(None) },
        )
        .await
        .unwrap()
        .expect("awaiting timeout");

    assert!(matches!(
        next_action,
        GapRecoveryAction::AwaitingTimeout(gap) if gap.expected_seq == 4 && gap.actual_seq == 8
    ));
}

#[tokio::test]
async fn test_gap_tracker_requests_rebuild_after_timeout_when_outbox_missing() {
    let tenant_id = TenantId::new("tenant-1").unwrap();
    let entity_id = Uuid::now_v7();
    let delivery = Delivery {
        delivery_id: "1-0".to_string(),
        stream_key: "tenant-1:events:0".to_string(),
        event: kernel_core::event::EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id,
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(8),
            },
            payload: Value::Null,
            created_at: chrono::Utc::now(),
            event_type: "entity.updated".to_string(),
        },
    };

    let start = Instant::now();
    let mut tracker = GapTracker::new(entity_ordering_key(&delivery.event.tenant_id, entity_id), 6);
    tracker.observe_delivery(&delivery, start).unwrap();

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
}

#[tokio::test]
async fn test_gap_tracker_defers_later_future_deliveries_while_gap_is_active() {
    let tenant_id = TenantId::new("tenant-1").unwrap();
    let entity_id = Uuid::now_v7();
    let mut tracker = GapTracker::new(entity_ordering_key(&tenant_id, entity_id), 3);
    let first_delivery = Delivery {
        delivery_id: "1-0".to_string(),
        stream_key: "tenant-1:events:0".to_string(),
        event: kernel_core::event::EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(8),
            },
            payload: Value::Null,
            created_at: chrono::Utc::now(),
            event_type: "entity.updated".to_string(),
        },
    };
    let second_delivery = Delivery {
        delivery_id: "2-0".to_string(),
        stream_key: "tenant-1:events:0".to_string(),
        event: kernel_core::event::EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id,
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(9),
            },
            payload: Value::Null,
            created_at: chrono::Utc::now(),
            event_type: "entity.updated".to_string(),
        },
    };

    let detected = tracker
        .observe_delivery(&first_delivery, Instant::now())
        .unwrap();
    assert!(matches!(
        detected,
        kernel_core::event::DeliveryResolution::GapDetected(_)
    ));

    let deferred = tracker
        .observe_delivery(&second_delivery, Instant::now())
        .unwrap();
    assert!(matches!(
        deferred,
        kernel_core::event::DeliveryResolution::Deferred(gap)
            if gap.expected_seq == 3 && gap.actual_seq == 9
    ));
}
