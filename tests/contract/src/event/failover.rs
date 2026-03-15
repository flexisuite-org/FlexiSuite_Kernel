use std::fs;
use std::hash::Hasher;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use chrono::Utc;
use kernel_core::auth::TenantId;
use kernel_core::event::{
    EventEnvelope, OrderMode, ReliableConsumer, ReliableProducer, SHARD_COUNT,
};
use kernel_data::event::{RedisConsumer, RedisProducer};
use twox_hash::XxHash64;
use uuid::Uuid;

struct RedisServerGuard {
    child: Child,
    data_dir: PathBuf,
}

impl Drop for RedisServerGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.data_dir);
    }
}

fn reserve_local_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().expect("local addr").port();
    drop(listener);
    port
}

async fn start_redis_server() -> (RedisServerGuard, redis::Client) {
    let port = reserve_local_port();
    let data_dir = PathBuf::from(format!("/tmp/flexisuite-redis-contract-{}", Uuid::now_v7()));
    fs::create_dir_all(&data_dir).expect("create redis temp dir");

    let child = Command::new("redis-server")
        .arg("--port")
        .arg(port.to_string())
        .arg("--bind")
        .arg("127.0.0.1")
        .arg("--save")
        .arg("")
        .arg("--appendonly")
        .arg("no")
        .arg("--dir")
        .arg(&data_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn redis-server");

    let guard = RedisServerGuard { child, data_dir };
    let client =
        redis::Client::open(format!("redis://127.0.0.1:{port}/")).expect("create redis client");

    for _ in 0..50 {
        if let Ok(mut conn) = client.get_multiplexed_async_connection().await {
            let ping: redis::RedisResult<String> = redis::cmd("PING").query_async(&mut conn).await;
            if ping.as_deref() == Ok("PONG") {
                return (guard, client);
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    panic!("redis-server did not become ready in time");
}

fn calculate_shard(key: &str) -> u64 {
    let mut hasher = XxHash64::default();
    hasher.write(key.as_bytes());
    hasher.finish() % SHARD_COUNT
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
    let first_shard = calculate_shard(
        &OrderMode::Entity {
            entity_id: first_entity_id,
            seq: Some(1),
        }
        .shard_input(tenant_id),
    );

    for _ in 0..1024 {
        let second_entity_id = Uuid::now_v7();
        let second_shard = calculate_shard(
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
    let shard = calculate_shard(&ordering_key);
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
        "backlog should remain visible after group creation"
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
