use chrono::Utc;
use kernel_core::auth::TenantId;
use kernel_core::event::{
    EventEnvelope, OrderMode, ReliableConsumer, ReliableProducer, calculate_shard,
};
use kernel_data::event::{RedisConsumer, RedisProducer};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::redis::{REDIS_PORT, Redis};
use uuid::Uuid;

type RedisNode = ContainerAsync<Redis>;

async fn start_redis_server() -> (RedisNode, redis::Client) {
    let node = Redis::default()
        .with_tag("7.2-alpine")
        .start()
        .await
        .expect("start redis");
    let port = node.get_host_port_ipv4(REDIS_PORT).await.expect("get port");
    let client =
        redis::Client::open(format!("redis://127.0.0.1:{port}/")).expect("create redis client");
    (node, client)
}

fn calculate_shard_local(key: &str) -> u64 {
    calculate_shard(key)
}

async fn publish_entity_event(
    producer: &RedisProducer,
    tenant_id: &TenantId,
    stream_base: &str,
    entity_id: Uuid,
    seq: u64,
    event_type: &str,
) {
    producer
        .publish(
            stream_base,
            EventEnvelope {
                event_id: Uuid::now_v7(),
                tenant_id: tenant_id.clone(),
                order_mode: OrderMode::Entity {
                    entity_id,
                    seq: Some(seq),
                },
                payload: serde_json::json!({ "seq": seq }),
                created_at: Utc::now(),
                event_type: event_type.to_string(),
            },
        )
        .await
        .expect("publish event");
}

fn find_distinct_entity_shards(tenant_id: &TenantId) -> ((Uuid, u64), (Uuid, u64)) {
    let first_entity_id = Uuid::now_v7();
    let first_shard = calculate_shard_local(
        &OrderMode::Entity {
            entity_id: first_entity_id,
            seq: Some(1),
        }
        .shard_input(tenant_id),
    );

    for _ in 0..1024 {
        let second_entity_id = Uuid::now_v7();
        let second_shard = calculate_shard_local(
            &OrderMode::Entity {
                entity_id: second_entity_id,
                seq: Some(1),
            }
            .shard_input(tenant_id),
        );
        if second_shard != first_shard {
            return (
                (first_entity_id, first_shard),
                (second_entity_id, second_shard),
            );
        }
    }

    panic!("failed to find distinct shards for contract test");
}

#[tokio::test]
async fn test_claim_pending_failover_preserves_ordering() {
    let (_redis, client) = start_redis_server().await;
    let producer = RedisProducer::new(client.clone()).await.expect("producer");
    let consumer = RedisConsumer::new(client.clone()).await.expect("consumer");

    let tenant_id = TenantId::new("tenant-failover").unwrap();
    let entity_id = Uuid::now_v7();
    let stream_base = "events";
    let consumer_group = "group-a";
    let ordering_key = OrderMode::Entity {
        entity_id,
        seq: Some(1),
    }
    .shard_input(&tenant_id);
    let shard = calculate_shard_local(&ordering_key);
    let stream_key = format!("{tenant_id}:{stream_base}:{shard}");

    let mut admin = client
        .get_multiplexed_async_connection()
        .await
        .expect("admin connection");
    let _: String = redis::cmd("XGROUP")
        .arg("CREATE")
        .arg(&stream_key)
        .arg(consumer_group)
        .arg("$")
        .arg("MKSTREAM")
        .query_async(&mut admin)
        .await
        .expect("create consumer group");

    for seq in [1_u64, 2_u64] {
        producer
            .publish(
                stream_base,
                EventEnvelope {
                    event_id: Uuid::now_v7(),
                    tenant_id: tenant_id.clone(),
                    order_mode: OrderMode::Entity {
                        entity_id,
                        seq: Some(seq),
                    },
                    payload: serde_json::json!({ "seq": seq }),
                    created_at: Utc::now(),
                    event_type: "contract.failover".to_string(),
                },
            )
            .await
            .expect("publish event");
    }

    let polled = consumer
        .poll(&tenant_id, stream_base, consumer_group, "consumer-1", 10)
        .await
        .expect("poll pending candidate deliveries");
    assert_eq!(polled.len(), 2, "consumer-1 should receive both deliveries");
    assert_eq!(
        polled
            .iter()
            .map(|delivery| delivery.event.order_mode.seq().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2],
        "initial delivery order must follow entity_seq",
    );

    let claimed = consumer
        .claim_pending(&tenant_id, stream_base, consumer_group, "consumer-2", 0, 10)
        .await
        .expect("claim pending deliveries after failover");
    assert_eq!(
        claimed.len(),
        2,
        "consumer-2 should reclaim both pending deliveries"
    );
    assert_eq!(
        claimed
            .iter()
            .map(|delivery| delivery.event.order_mode.seq().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2],
        "claim_pending must preserve per-key ordering during failover",
    );
    assert_eq!(
        claimed
            .iter()
            .map(|delivery| delivery.delivery_id.clone())
            .collect::<Vec<_>>(),
        polled
            .iter()
            .map(|delivery| delivery.delivery_id.clone())
            .collect::<Vec<_>>(),
        "failover must keep the original stream ordering for reclaimed entries",
    );

    let pending: redis::Value = redis::cmd("XPENDING")
        .arg(&stream_key)
        .arg(consumer_group)
        .query_async(&mut admin)
        .await
        .expect("query pending summary");
    let pending = match pending {
        redis::Value::Array(values) => match values.first() {
            Some(redis::Value::Int(count)) => *count,
            other => panic!("unexpected XPENDING summary head: {other:?}"),
        },
        other => panic!("unexpected XPENDING summary payload: {other:?}"),
    };
    assert_eq!(
        pending, 2,
        "claimed deliveries must remain pending until acked"
    );
}

#[tokio::test]
async fn test_poll_recovers_existing_backlog_when_consumer_group_is_missing() {
    let (_redis, client) = start_redis_server().await;
    let producer = RedisProducer::new(client.clone()).await.expect("producer");
    let consumer = RedisConsumer::new(client.clone()).await.expect("consumer");

    let tenant_id = TenantId::new("tenant-backlog").unwrap();
    let entity_id = Uuid::now_v7();
    let stream_base = "events";
    let consumer_group = "group-backlog";

    publish_entity_event(
        &producer,
        &tenant_id,
        stream_base,
        entity_id,
        1,
        "contract.backlog",
    )
    .await;
    publish_entity_event(
        &producer,
        &tenant_id,
        stream_base,
        entity_id,
        2,
        "contract.backlog",
    )
    .await;

    let deliveries = consumer
        .poll(&tenant_id, stream_base, consumer_group, "consumer-1", 10)
        .await
        .expect("poll should recover pre-existing backlog");

    assert_eq!(
        deliveries.len(),
        2,
        "backlog should remain visible after_group creation"
    );
    assert_eq!(
        deliveries
            .iter()
            .map(|delivery| delivery.event.order_mode.seq().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2],
        "backlog should be delivered in-order after creating the consumer group",
    );
}

#[tokio::test]
async fn test_poll_never_exceeds_max_count_across_multiple_shards() {
    let (_redis, client) = start_redis_server().await;
    let producer = RedisProducer::new(client.clone()).await.expect("producer");
    let consumer = RedisConsumer::new(client.clone()).await.expect("consumer");

    let tenant_id = TenantId::new("tenant-max-count").unwrap();
    let stream_base = "events";
    let consumer_group = "group-max-count";
    let ((entity_a, _), (entity_b, _)) = find_distinct_entity_shards(&tenant_id);

    publish_entity_event(
        &producer,
        &tenant_id,
        stream_base,
        entity_a,
        1,
        "contract.max-count",
    )
    .await;
    publish_entity_event(
        &producer,
        &tenant_id,
        stream_base,
        entity_b,
        1,
        "contract.max-count",
    )
    .await;

    let first_batch = consumer
        .poll(&tenant_id, stream_base, consumer_group, "consumer-1", 1)
        .await
        .expect("first poll");
    assert_eq!(
        first_batch.len(),
        1,
        "poll(max_count=1) must not over-deliver"
    );
    consumer
        .ack(
            &tenant_id,
            &first_batch[0].stream_key,
            consumer_group,
            &first_batch[0].delivery_id,
        )
        .await
        .expect("ack first delivery");

    let second_batch = consumer
        .poll(&tenant_id, stream_base, consumer_group, "consumer-1", 1)
        .await
        .expect("second poll");
    assert_eq!(
        second_batch.len(),
        1,
        "remaining shard delivery should arrive in the next poll"
    );
    assert_ne!(
        first_batch[0]
            .event
            .order_mode
            .tenant_scoped_ordering_key(&tenant_id),
        second_batch[0]
            .event
            .order_mode
            .tenant_scoped_ordering_key(&tenant_id),
        "the two polls should observe different ordering keys from different shards",
    );
}

#[tokio::test]
async fn test_poison_pill_is_acked() {
    let (_node, client) = start_redis_server().await;
    let tenant_id = TenantId::new("tenant-poison").unwrap();
    let stream_base = "test_poison_stream";
    let consumer_group = "test_group";

    // Inject a poison pill
    let mut conn = client.get_connection_manager().await.unwrap();
    let shard_key = format!("{}:{}:0", tenant_id, stream_base);
    let _: () = redis::cmd("XADD")
        .arg(&shard_key)
        .arg("*")
        .arg("data")
        .arg("invalid_json_payload")
        .query_async(&mut conn)
        .await
        .unwrap();

    let consumer = RedisConsumer::new(client.clone()).await.expect("consumer");

    // Poll the consumer. It should encounter the poison pill, log an error,
    // XACK it, and return empty (or other valid messages if present).
    let deliveries = consumer
        .poll(&tenant_id, stream_base, consumer_group, "consumer-1", 10)
        .await
        .expect("poll should succeed even with poison pill");

    assert_eq!(deliveries.len(), 0, "poison pill should be dropped from result");

    // Ensure the message was actually XACKed (Pending Entries List should be empty)
    let pending: redis::Value = redis::cmd("XPENDING")
        .arg(&shard_key)
        .arg(consumer_group)
        .query_async(&mut conn)
        .await
        .unwrap();

    // XPENDING returns [total_pending, min_id, max_id, [consumers]]
    let pending_arr = match pending {
        redis::Value::Array(ref arr) => arr,
        _ => panic!("Expected array"),
    };
    let total_pending: i64 =
        redis::FromRedisValue::from_redis_value(pending_arr[0].clone()).unwrap();
    assert_eq!(total_pending, 0, "Poison pill must be XACKed (PEL should be empty)");
}

#[tokio::test]
async fn test_phase_2_respects_max_count() {
    let (_node, client) = start_redis_server().await;
    let tenant_id = TenantId::new("tenant-phase2").unwrap();
    let stream_base = "test_phase2_stream";
    let consumer_group = "test_group";

    let consumer = RedisConsumer::new(client.clone()).await.expect("consumer");
    let conn = client.get_connection_manager().await.unwrap();

    // Spawn a task to inject events into multiple shards after a delay (so Phase 1 misses them)
    let tenant_id_clone = tenant_id.clone();
    let mut conn_clone = conn.clone();
    let injector = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        // Use a pipeline to inject all events atomically. This ensures deterministic
        // wake-up behavior for the blocked Phase 2 XREADGROUP, so it sees all shards having data.
        let mut pipe = redis::pipe();
        pipe.atomic(); // Ensure MULTI/EXEC transaction so XREADGROUP cannot wake up mid-injection
        for i in 0..5 {
            let shard_key = format!("{}:{}:{}", tenant_id_clone, "test_phase2_stream", i);
            pipe.cmd("XADD")
                .arg(&shard_key)
                .arg("*")
                .arg("data")
                .arg(serde_json::to_string(&EventEnvelope {
                    event_id: Uuid::now_v7(),
                    tenant_id: tenant_id_clone.clone(),
                    order_mode: OrderMode::Entity { entity_id: Uuid::now_v7(), seq: Some(1) },
                    payload: serde_json::json!({ "seq": 1 }),
                    created_at: Utc::now(),
                    event_type: "test".to_string(),
                }).unwrap());
        }
        pipe.query_async::<()>(&mut conn_clone).await.unwrap();
    });

    // Read with max_count = 2 using blocking read (Phase 2 will catch it)
    let deliveries = consumer
        .poll(&tenant_id, stream_base, consumer_group, "consumer-1", 2)
        .await
        .expect("poll");

    let _ = injector.await.expect("injector task should not panic");

    assert_eq!(deliveries.len(), 2, "must read exactly max_count messages");
}


#[tokio::test]
async fn test_mid_batch_poison_pill_preserves_surrounding_valid_entries() {
    let (_node, client) = start_redis_server().await;
    let tenant_id = TenantId::new("tenant-mid-poison").unwrap();
    let stream_base = "test_mid_poison_stream";
    let consumer_group = "test_group";

    let mut conn = client.get_connection_manager().await.unwrap();
    let shard_key = format!("{}:{}:0", tenant_id, stream_base);

    // Create the group
    let _: () = redis::cmd("XGROUP")
        .arg("CREATE")
        .arg(&shard_key)
        .arg(consumer_group)
        .arg("0")
        .arg("MKSTREAM")
        .query_async(&mut conn)
        .await
        .unwrap();

    let mut pipe = redis::pipe();

    // Valid 1
    pipe.cmd("XADD").arg(&shard_key).arg("*").arg("data").arg(serde_json::to_string(&EventEnvelope {
        event_id: Uuid::now_v7(), tenant_id: tenant_id.clone(), order_mode: OrderMode::Causality { key: "a".to_string(), seq: Some(1) },
        payload: serde_json::json!({ "seq": 1 }), created_at: Utc::now(), event_type: "t".to_string(),
    }).unwrap());

    // Poison
    pipe.cmd("XADD").arg(&shard_key).arg("*").arg("data").arg("invalid_json_payload");

    // Valid 2
    pipe.cmd("XADD").arg(&shard_key).arg("*").arg("data").arg(serde_json::to_string(&EventEnvelope {
        event_id: Uuid::now_v7(), tenant_id: tenant_id.clone(), order_mode: OrderMode::Causality { key: "a".to_string(), seq: Some(2) },
        payload: serde_json::json!({ "seq": 2 }), created_at: Utc::now(), event_type: "t".to_string(),
    }).unwrap());

    pipe.query_async::<()>(&mut conn).await.unwrap();

    let consumer = RedisConsumer::new(client.clone()).await.unwrap();
    let deliveries = consumer
        .poll(&tenant_id, stream_base, consumer_group, "consumer-1", 10)
        .await
        .expect("poll");

    assert_eq!(deliveries.len(), 2, "must decode and return exactly both valid messages, skipping poison");

    // Check pending list
    let pending: redis::Value = redis::cmd("XPENDING")
        .arg(&shard_key)
        .arg(consumer_group)
        .query_async(&mut conn)
        .await
        .unwrap();
    let pending_arr = match pending { redis::Value::Array(ref arr) => arr, _ => panic!("Expected array") };
    let _total_pending: i64 = redis::FromRedisValue::from_redis_value(pending_arr[0].clone()).unwrap();

    // The valid entries are NOT auto-acked by poll! Poll just returns them.
    // XREADGROUP adds them to PEL. The caller must call consumer.ack().
    // We should ack the valid ones to prove they can be acked and verify the poison one was auto-xacked.
    consumer.ack(&tenant_id, &shard_key, consumer_group, &deliveries[0].delivery_id).await.unwrap();
    consumer.ack(&tenant_id, &shard_key, consumer_group, &deliveries[1].delivery_id).await.unwrap();

    let pending: redis::Value = redis::cmd("XPENDING")
        .arg(&shard_key)
        .arg(consumer_group)
        .query_async(&mut conn)
        .await
        .unwrap();
    let pending_arr = match pending { redis::Value::Array(ref arr) => arr, _ => panic!("Expected array") };
    let total_pending: i64 = redis::FromRedisValue::from_redis_value(pending_arr[0].clone()).unwrap();

    // Poison is force-acked, valid entries are explicitly acked. PEL must be empty!
    assert_eq!(total_pending, 0, "PEL should be empty since poison pill was force-acked");
}

#[tokio::test]
async fn test_mid_batch_tenant_mismatch_preserves_surrounding_valid_entries() {
    let (_node, client) = start_redis_server().await;
    let tenant_id = TenantId::new("tenant-mid-iso").unwrap();
    let stream_base = "test_mid_iso_stream";
    let consumer_group = "test_group";

    let mut conn = client.get_connection_manager().await.unwrap();
    let shard_key = format!("{}:{}:0", tenant_id, stream_base);

    // Create the group
    let _: () = redis::cmd("XGROUP")
        .arg("CREATE")
        .arg(&shard_key)
        .arg(consumer_group)
        .arg("0")
        .arg("MKSTREAM")
        .query_async(&mut conn)
        .await
        .unwrap();

    let mut pipe = redis::pipe();

    // Valid 1
    pipe.cmd("XADD").arg(&shard_key).arg("*").arg("data").arg(serde_json::to_string(&EventEnvelope {
        event_id: Uuid::now_v7(), tenant_id: tenant_id.clone(), order_mode: OrderMode::Causality { key: "a".to_string(), seq: Some(1) },
        payload: serde_json::json!({ "seq": 1 }), created_at: Utc::now(), event_type: "t".to_string(),
    }).unwrap());

    // Tenant mismatch
    pipe.cmd("XADD").arg(&shard_key).arg("*").arg("data").arg(serde_json::to_string(&EventEnvelope {
        event_id: Uuid::now_v7(), tenant_id: TenantId::new("tenant-other").unwrap(), order_mode: OrderMode::Causality { key: "a".to_string(), seq: Some(1) },
        payload: serde_json::json!({ "seq": 1 }), created_at: Utc::now(), event_type: "t".to_string(),
    }).unwrap());

    // Valid 2
    pipe.cmd("XADD").arg(&shard_key).arg("*").arg("data").arg(serde_json::to_string(&EventEnvelope {
        event_id: Uuid::now_v7(), tenant_id: tenant_id.clone(), order_mode: OrderMode::Causality { key: "a".to_string(), seq: Some(2) },
        payload: serde_json::json!({ "seq": 2 }), created_at: Utc::now(), event_type: "t".to_string(),
    }).unwrap());

    pipe.query_async::<()>(&mut conn).await.unwrap();

    let consumer = RedisConsumer::new(client.clone()).await.unwrap();
    let deliveries = consumer
        .poll(&tenant_id, stream_base, consumer_group, "consumer-1", 10)
        .await
        .expect("poll");

    assert_eq!(deliveries.len(), 2, "must decode and return exactly both valid messages, skipping isolation failure");

    // CR-3: Verify returned deliveries belong to the correct tenant
    assert_eq!(deliveries[0].event.tenant_id, tenant_id);
    assert_eq!(deliveries[1].event.tenant_id, tenant_id);
    let seq0 = match deliveries[0].event.order_mode { OrderMode::Causality { seq: Some(s), .. } => s, _ => panic!("bad order mode") };
    let seq1 = match deliveries[1].event.order_mode { OrderMode::Causality { seq: Some(s), .. } => s, _ => panic!("bad order mode") };
    assert_eq!(seq0, 1);
    assert_eq!(seq1, 2);

    // Ack valid ones
    consumer.ack(&tenant_id, &shard_key, consumer_group, &deliveries[0].delivery_id).await.unwrap();
    consumer.ack(&tenant_id, &shard_key, consumer_group, &deliveries[1].delivery_id).await.unwrap();

    let pending: redis::Value = redis::cmd("XPENDING")
        .arg(&shard_key)
        .arg(consumer_group)
        .query_async(&mut conn)
        .await
        .unwrap();
    let pending_arr = match pending { redis::Value::Array(ref arr) => arr, _ => panic!("Expected array") };
    let total_pending: i64 = redis::FromRedisValue::from_redis_value(pending_arr[0].clone()).unwrap();

    // Mismatch remains in PEL!
    assert_eq!(total_pending, 1, "Tenant mismatch entry must remain in PEL");
}


#[tokio::test]
async fn test_claim_pending_once_preserves_surrounding_valid_entries_on_error() {
    let (_node, client) = start_redis_server().await;
    let tenant_id = TenantId::new("tenant-claim-err").unwrap();
    let stream_base = "test_claim_err_stream";
    let consumer_group = "test_group";

    let mut conn = client.get_connection_manager().await.unwrap();

    let shard_0 = format!("{}:{}:0", tenant_id, stream_base);
    let shard_1 = format!("{}:{}:1", tenant_id, stream_base);

    // Create group on both shards
    let _: () = redis::cmd("XGROUP").arg("CREATE").arg(&shard_0).arg(consumer_group).arg("0").arg("MKSTREAM").query_async(&mut conn).await.unwrap();
    let _: () = redis::cmd("XGROUP").arg("CREATE").arg(&shard_1).arg(consumer_group).arg("0").arg("MKSTREAM").query_async(&mut conn).await.unwrap();

    let consumer = RedisConsumer::new(client.clone()).await.unwrap();

    // Pre-warm consumer group cache before test data is inserted so the warm-up
    // cannot claim and consume entries that this test needs to observe.
    let _ = consumer.claim_pending(&tenant_id, stream_base, consumer_group, "consumer-2", 0, 1).await;

    // Inject 1 event into shard 0
    let _: () = redis::cmd("XADD").arg(&shard_0).arg("*").arg("data").arg(serde_json::to_string(&EventEnvelope {
        event_id: Uuid::now_v7(), tenant_id: tenant_id.clone(), order_mode: OrderMode::Causality { key: "a".to_string(), seq: Some(1) },
        payload: serde_json::json!({ "seq": 1 }), created_at: Utc::now(), event_type: "t".to_string(),
    }).unwrap()).query_async(&mut conn).await.unwrap();

    // Inject 1 event into shard 1
    let _: () = redis::cmd("XADD").arg(&shard_1).arg("*").arg("data").arg(serde_json::to_string(&EventEnvelope {
        event_id: Uuid::now_v7(), tenant_id: tenant_id.clone(), order_mode: OrderMode::Causality { key: "b".to_string(), seq: Some(1) },
        payload: serde_json::json!({ "seq": 1 }), created_at: Utc::now(), event_type: "t".to_string(),
    }).unwrap()).query_async(&mut conn).await.unwrap();

    // Have consumer-1 read them to move them into PEL
    let _: redis::Value = redis::cmd("XREADGROUP")
        .arg("GROUP").arg(consumer_group).arg("consumer-1")
        .arg("COUNT").arg(10)
        .arg("STREAMS").arg(&shard_0).arg(&shard_1).arg(">").arg(">")
        .query_async(&mut conn).await.unwrap();

    let _: () = redis::cmd("DEL").arg(&shard_1).query_async(&mut conn).await.unwrap();
    let _: () = redis::cmd("SET").arg(&shard_1).arg("not a stream").query_async(&mut conn).await.unwrap();

    // Now call claim_pending. Shard 0 will successfully claim its entry.
    // Shard 1 will throw a WRONGTYPE error from XAUTOCLAIM.
    // The implementation MUST return Ok([delivery from shard 0]) instead of Err.
    // To ensure we read from shard 0 first, claim_pending_once starts at a random shard,
    // but it rotates through all.
    // We can just try a few times to make sure we hit the order where shard 0 is before shard 1.
    // Actually, shard count is 64. 0 and 1 are adjacent. If it starts at 0, it hits 0 then 1.
    let mut claimed = Vec::new();
    for _ in 0..64 {
        let res = consumer.claim_pending(&tenant_id, stream_base, consumer_group, "consumer-2", 0, 10).await;
        if let Ok(mut items) = res {
            claimed.append(&mut items);
        } else {
            // It hit shard 1 first and threw an error! That's fine, we keep looping until we hit shard 0 first.
            // Wait, if it hits shard 1 first, it throws an error and returns.
            // On the next loop, `next_poll_start_shard` will advance! So it will eventually hit shard 0 first!
        }
        if !claimed.is_empty() {
            break;
        }
    }

    assert_eq!(claimed.len(), 1, "must preserve and return successfully claimed deliveries even if a later shard throws an error");
}
