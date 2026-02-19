use chrono::Utc;
use kernel_core::auth::TenantId;
use kernel_core::event::{
    EventEnvelope, OrderMode, compare_event_order, validate_order_mode_transition,
};
use serde_json::json;
use uuid::Uuid;

#[tokio::test]
async fn test_event_mode_mix_forbidden() {
    let entity_id = Uuid::now_v7();
    let existing = OrderMode::Entity {
        entity_id,
        seq: Some(1),
    };
    let next = OrderMode::Causality {
        key: entity_id.to_string(),
        seq: Some(2),
    };

    let res = validate_order_mode_transition(Some(&existing), &next);
    assert!(
        res.is_err(),
        "Must reject mixed order_mode for same entity_id"
    );
}

#[tokio::test]
async fn test_event_ordering_entity() {
    let tenant_id = TenantId::new("tenant-event").unwrap();
    let entity_id = Uuid::now_v7();
    let mut events = vec![2_u64, 1, 3]
        .into_iter()
        .map(|seq| EventEnvelope {
            event_id: Uuid::now_v7(),
            tenant_id: tenant_id.clone(),
            order_mode: OrderMode::Entity {
                entity_id,
                seq: Some(seq),
            },
            payload: json!({"seq": seq}),
            created_at: Utc::now(),
            event_type: "contract.event".to_string(),
        })
        .collect::<Vec<_>>();

    // Use partial_cmp-aware sorting with a deterministic tiebreaker (event_id for incomparable)
    events.sort_by(|a, b| {
        compare_event_order(a, b).unwrap_or_else(|| a.event_id.cmp(&b.event_id))
    });

    assert_eq!(events[0].order_mode.seq(), Some(1));
    assert_eq!(events[1].order_mode.seq(), Some(2));
    assert_eq!(events[2].order_mode.seq(), Some(3));
}
