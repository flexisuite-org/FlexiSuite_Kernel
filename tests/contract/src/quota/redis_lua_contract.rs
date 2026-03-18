#[cfg(test)]
mod tests {
    use kernel_api::middleware::{QuotaConfig, QuotaLayerConfig, RedisQuotaStore, QuotaStore, violation_to_response};
    use kernel_core::auth::TenantId;
    use kernel_core::quota::QuotaLayer;
    use testcontainers::runners::AsyncRunner;
    use testcontainers::{ContainerAsync, ImageExt};
    use testcontainers_modules::redis::{REDIS_PORT, Redis};

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

    #[tokio::test]
    async fn test_redis_lua_circuit_breaker_contract() {
        let (_node, client) = start_redis_server().await;
        let manager = redis::aio::ConnectionManager::new(client)
            .await
            .expect("create connection manager");

        let quota_config = QuotaConfig {
            system_hard_limit: QuotaLayerConfig { rate: 1000.0, capacity: 1000.0, cost: 1.0, backoff_s: 30.0 },
            tenant_budget: QuotaLayerConfig { rate: 100.0, capacity: 300.0, cost: 1.0, backoff_s: 30.0 },
            api_rate_limit: QuotaLayerConfig { rate: 10.0, capacity: 50.0, cost: 1.0, backoff_s: 30.0 },
            circuit_breaker: QuotaLayerConfig {
                rate: 1.0,
                capacity: 5.0,
                cost: 1.0,
                backoff_s: 1.0,
            },
            tenant_overrides: std::collections::HashMap::new(),
        };

        let store = RedisQuotaStore::new(manager, quota_config);
        let tenant_id = TenantId::new("test-tenant").unwrap();

        // 1. Consume all tokens
        for _ in 0..5 {
            assert!(store.check_and_update(&tenant_id, QuotaLayer::CircuitBreaker).await.is_ok());
        }

        // 2. Trip
        let res = store.check_and_update(&tenant_id, QuotaLayer::CircuitBreaker).await;
        assert!(res.is_err());
        let violation = res.unwrap_err();
        assert_eq!(violation.layer, QuotaLayer::CircuitBreaker);
        assert_eq!(violation.retry_after_s, 1);

        // Verify X-Violation-Type through the authoritative response mapper (Codex P2)
        let response = violation_to_response(&violation);
        assert_eq!(
            response.headers().get("X-Violation-Type").unwrap().to_str().unwrap(),
            QuotaLayer::CircuitBreaker.violation_type()
        );

        // 3. Recovery
        tokio::time::sleep(tokio::time::Duration::from_millis(1100)).await;
        assert!(store.check_and_update(&tenant_id, QuotaLayer::CircuitBreaker).await.is_ok());
    }

    #[tokio::test]
    async fn test_redis_lua_multi_script_circuit_breaker_contract() {
        let (_node, client) = start_redis_server().await;
        let manager = redis::aio::ConnectionManager::new(client)
            .await
            .expect("create connection manager");

        let quota_config = QuotaConfig {
            system_hard_limit: QuotaLayerConfig { rate: 1000.0, capacity: 1000.0, cost: 1.0, backoff_s: 30.0 },
            tenant_budget: QuotaLayerConfig { rate: 100.0, capacity: 300.0, cost: 1.0, backoff_s: 30.0 },
            api_rate_limit: QuotaLayerConfig { rate: 10.0, capacity: 50.0, cost: 1.0, backoff_s: 30.0 },
            circuit_breaker: QuotaLayerConfig {
                rate: 1.0,
                capacity: 5.0,
                cost: 1.0,
                backoff_s: 1.0,
            },
            tenant_overrides: std::collections::HashMap::new(),
        };

        let store = RedisQuotaStore::new(manager, quota_config);
        let tenant_id = TenantId::new("test-multi-tenant").unwrap();

        let layers = vec![
            QuotaLayer::SystemHardLimit,
            QuotaLayer::CircuitBreaker,
            QuotaLayer::TenantBudget,
            QuotaLayer::ApiRateLimit,
        ];

        // 1. Consume all CB tokens (SHL has plenty)
        for _ in 0..5 {
            assert!(store.check_and_update_multi(&tenant_id, &layers).await.is_ok());
        }

        // 2. Trip CB
        let res = store.check_and_update_multi(&tenant_id, &layers).await;
        assert!(res.is_err());
        let violation = res.unwrap_err();
        assert_eq!(violation.layer, QuotaLayer::CircuitBreaker);
        assert_eq!(violation.retry_after_s, 1);

        // Verify X-Violation-Type through the authoritative response mapper (Codex P2)
        let response = violation_to_response(&violation);
        assert_eq!(
            response.headers().get("X-Violation-Type").unwrap().to_str().unwrap(),
            QuotaLayer::CircuitBreaker.violation_type()
        );

        // 3. Recovery
        tokio::time::sleep(tokio::time::Duration::from_millis(1100)).await;
        assert!(store.check_and_update_multi(&tenant_id, &layers).await.is_ok());
    }

    #[tokio::test]
    async fn test_redis_lua_priority_contract() {
        let (_node, client) = start_redis_server().await;
        let manager = redis::aio::ConnectionManager::new(client)
            .await
            .expect("create connection manager");

        let quota_config = QuotaConfig {
            system_hard_limit: QuotaLayerConfig { rate: 1.0, capacity: 1.0, cost: 1.0, backoff_s: 30.0 },
            tenant_budget: QuotaLayerConfig { rate: 100.0, capacity: 100.0, cost: 1.0, backoff_s: 30.0 },
            api_rate_limit: QuotaLayerConfig { rate: 100.0, capacity: 100.0, cost: 1.0, backoff_s: 30.0 },
            circuit_breaker: QuotaLayerConfig { rate: 1.0, capacity: 1.0, cost: 1.0, backoff_s: 30.0 },
            tenant_overrides: std::collections::HashMap::new(),
        };

        let store = RedisQuotaStore::new(manager, quota_config);
        let tenant_id = TenantId::new("test-priority-tenant").unwrap();

        let layers = vec![
            QuotaLayer::SystemHardLimit,
            QuotaLayer::CircuitBreaker,
        ];

        // 1. First request: OK
        assert!(store.check_and_update_multi(&tenant_id, &layers).await.is_ok());
        
        // 2. Second request: Both would fail, but SHL is higher priority
        let res = store.check_and_update_multi(&tenant_id, &layers).await;
        assert!(res.is_err());
        let violation = res.unwrap_err();
        assert_eq!(violation.layer, QuotaLayer::SystemHardLimit, "SHL must win over CB");

        // Verify X-Violation-Type through the authoritative response mapper (Codex P2)
        let response = violation_to_response(&violation);
        assert_eq!(
            response.headers().get("X-Violation-Type").unwrap().to_str().unwrap(),
            QuotaLayer::SystemHardLimit.violation_type()
        );
    }
}
