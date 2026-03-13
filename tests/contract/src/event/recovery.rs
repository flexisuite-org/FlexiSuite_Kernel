use kernel_core::auth::TenantId;
use kernel_core::event::{
    Delivery, GapRecoveryAction, GapRecoveryState, GapTracker, OrderMode, progress_gap_recovery,
};
use serde_json::Value;
use std::time::{Duration, Instant};
use uuid::Uuid;

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
    let mut tracker = GapTracker::new(3);
    tracker.observe_delivery(&delivery, start).unwrap();

    let action = tracker
        .recover_gap(start + Duration::from_secs(31), Duration::from_secs(30), |_| async {
            Ok(Some(missing))
        })
        .await
        .unwrap()
        .expect("recovery action");

    assert!(matches!(action, GapRecoveryAction::ReplayMissing(_)));
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
    let mut tracker = GapTracker::new(6);
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
        GapRecoveryAction::MarkPoisonAndRebuild { ordering_key } if ordering_key == entity_id.to_string()
    ));
}
