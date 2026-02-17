#![allow(dead_code)]
#![allow(unused_imports)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};

// --- Contract Definitions (from docs/implementation_plan.md) ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OrderMode {
    Entity,
    Causality,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: String,
    pub entity_id: String,
    pub order_mode: OrderMode,
    // For Entity mode
    pub entity_seq: Option<u64>,
    // For Causality mode
    pub causality_key: Option<String>,
    pub causality_seq: Option<u64>,

    pub payload: String,
}

#[derive(Debug, Clone)]
pub struct Delivery {
    pub delivery_id: String,
    pub envelope: EventEnvelope,
}

#[async_trait]
pub trait ReliableConsumer: Send + Sync {
    async fn poll(&self, stream: &str, consumer: &str, max_count: usize) -> Vec<Delivery>;
    async fn ack(&self, stream: &str, delivery_id: &str);
}

#[async_trait]
pub trait ReliableProducer: Send + Sync {
    async fn publish(&self, stream: &str, event: EventEnvelope) -> Result<String, String>;
}

// --- Mock Implementation for Contract Verification ---

struct MockEventSystem {
    // Stream -> List of events
    streams: Mutex<HashMap<String, VecDeque<EventEnvelope>>>,
    // Entity ID -> OrderMode (to enforcing mixing constraint)
    entity_modes: Mutex<HashMap<String, OrderMode>>,
}

impl MockEventSystem {
    fn new() -> Self {
        Self {
            streams: Mutex::new(HashMap::new()),
            entity_modes: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ReliableProducer for MockEventSystem {
    async fn publish(&self, stream: &str, event: EventEnvelope) -> Result<String, String> {
        let mut modes = self.entity_modes.lock().unwrap();

        // Contract: Mixed modes forbidden
        if let Some(existing_mode) = modes.get(&event.entity_id) {
            if *existing_mode != event.order_mode {
                return Err("Mixed order_mode forbidden".to_string());
            }
        } else {
            modes.insert(event.entity_id.clone(), event.order_mode.clone());
        }

        // Contract: Mandatory fields per mode
        match event.order_mode {
            OrderMode::Entity => {
                if event.entity_seq.is_none() {
                    return Err("entity_seq missing for Entity mode".to_string());
                }
            }
            OrderMode::Causality => {
                if event.causality_key.is_none() || event.causality_seq.is_none() {
                    return Err("causality_key/seq missing for Causality mode".to_string());
                }
            }
        }

        let mut streams = self.streams.lock().unwrap();
        streams.entry(stream.to_string()).or_default().push_back(event);
        Ok("ok".to_string())
    }
}

#[async_trait]
impl ReliableConsumer for MockEventSystem {
    async fn poll(&self, stream: &str, _consumer: &str, max_count: usize) -> Vec<Delivery> {
        let mut streams = self.streams.lock().unwrap();
        if let Some(queue) = streams.get_mut(stream) {
            let mut deliveries = Vec::new();
            for _ in 0..max_count {
                if let Some(env) = queue.pop_front() {
                     deliveries.push(Delivery {
                         delivery_id: format!("{}-d", env.event_id),
                         envelope: env,
                     });
                } else {
                    break;
                }
            }
            deliveries
        } else {
            Vec::new()
        }
    }

    async fn ack(&self, _stream: &str, _delivery_id: &str) {
        // No-op for mock
    }
}


#[tokio::test]
async fn test_event_mode_mix_forbidden() {
    let sys = MockEventSystem::new();
    let stream = "events:shard-1";

    let event1 = EventEnvelope {
        event_id: "e1".to_string(),
        entity_id: "entity-A".to_string(),
        order_mode: OrderMode::Entity,
        entity_seq: Some(1),
        causality_key: None,
        causality_seq: None,
        payload: "p1".to_string(),
    };

    assert!(sys.publish(stream, event1).await.is_ok());

    let event2 = EventEnvelope {
        event_id: "e2".to_string(),
        entity_id: "entity-A".to_string(),
        order_mode: OrderMode::Causality, // MIXED MODE!
        entity_seq: None,
        causality_key: Some("key-A".to_string()),
        causality_seq: Some(1),
        payload: "p2".to_string(),
    };

    let res = sys.publish(stream, event2).await;
    assert!(res.is_err(), "Must reject mixed order_mode for same entity_id");
}

#[tokio::test]
async fn test_event_ordering_entity() {
    let sys = Arc::new(MockEventSystem::new());
    let stream = "events:shard-1";

    // Publish out of order (simulating arrival time diff or just verifying consumer reordering if implemented?
    // Usually producer sends in order of commit. If producer sends 1 then 2, consumer reads 1 then 2.
    // The test ensures that if we publish 1, 2, 3, we consume 1, 2, 3.
    // Also "entity_seq" is the logical clock.

    for i in 1..=3 {
        let event = EventEnvelope {
            event_id: format!("e{}", i),
            entity_id: "entity-B".to_string(),
            order_mode: OrderMode::Entity,
            entity_seq: Some(i),
            causality_key: None,
            causality_seq: None,
            payload: format!("p{}", i),
        };
        sys.publish(stream, event).await.unwrap();
    }

    let deliveries = sys.poll(stream, "c1", 10).await;
    assert_eq!(deliveries.len(), 3);
    assert_eq!(deliveries[0].envelope.entity_seq, Some(1));
    assert_eq!(deliveries[1].envelope.entity_seq, Some(2));
    assert_eq!(deliveries[2].envelope.entity_seq, Some(3));
}
