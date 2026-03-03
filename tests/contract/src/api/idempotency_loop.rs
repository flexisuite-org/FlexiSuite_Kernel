use axum::{
    Extension, Router,
    body::{Body, Bytes, to_bytes},
    http::{HeaderValue, Request, StatusCode},
    middleware,
    response::Response,
    routing::post,
};
use kernel_api::auth::{TenantContext, TenantId, UserId};
use kernel_api::middleware::{
    IdempotencyEntry, IdempotencyScopeKey, IdempotencyStore, InMemoryActionStore,
    InMemoryIdempotencyStore, InMemoryQuotaStore, MiddlewareConfig, MiddlewareState,
    idempotency_middleware,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::task::JoinSet;
use tower::ServiceExt; // for oneshot

#[tokio::test]
async fn test_idempotency_loop_limit() {
    // Setup
    let config = MiddlewareConfig {
        idempotency_ttl: Duration::from_secs(60),
        // Large enough to avoid timeout, so we hit the loop limit instead
        inflight_wait_timeout: Duration::from_millis(1000),
        require_redis: false,
        ..Default::default()
    };
    let state = MiddlewareState::new(config)
        .await
        .expect("middleware state");

    // Mock handler that simulates FAST processing time but FAILS.
    // Failure causes the lock to be released (instead of Completed), allowing others to try acquire.
    // This creates the race condition where a waiter can repeatedly lose the race.
    let app = Router::new()
        .route(
            "/",
            post(|_: Request<Body>| async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                StatusCode::INTERNAL_SERVER_ERROR // Force release_inflight
            }),
        )
        .layer(middleware::from_fn(idempotency_middleware))
        .layer(Extension(state));

    let mut set = JoinSet::new();
    let num_requests = 20;
    let tenant_ctx = TenantContext::new(
        TenantId::new("tenant-1").unwrap(),
        Some(UserId::new("user-1").unwrap()),
    );

    for _ in 0..num_requests {
        let app = app.clone();
        let ctx = tenant_ctx.clone();
        set.spawn(async move {
            let req = Request::builder()
                .method("POST")
                .uri("/")
                .header("Idempotency-Key", "test-key-loop-v4")
                .extension(ctx)
                .body(Body::from("same-body"))
                .unwrap();

            match app.oneshot(req).await {
                Ok(res) => res.status(),
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
            }
        });
    }

    let mut service_unavailable_count = 0; // 503
    let mut _internal_error_count = 0; // 500 (from handler)
    let mut _conflict_count = 0; // 409
    let mut _other_count = 0;

    while let Some(res) = set.join_next().await {
        let status = res.unwrap();
        if status == StatusCode::SERVICE_UNAVAILABLE {
            service_unavailable_count += 1;
        } else if status == StatusCode::INTERNAL_SERVER_ERROR {
            _internal_error_count += 1;
        } else if status == StatusCode::CONFLICT {
            _conflict_count += 1;
        } else {
            _other_count += 1;
        }
    }

    assert_eq!(
        _other_count, 0,
        "Unexpected status responses detected: {}",
        _other_count
    );
    assert_eq!(
        service_unavailable_count + _internal_error_count + _conflict_count,
        num_requests,
        "Missing responses from idempotency loop"
    );

    // We expect some requests to process (return 500) and some to fail acquiring (return 503).
    assert!(
        service_unavailable_count > 0,
        "Expected at least one 503 due to loop limit"
    );
}

#[tokio::test]
async fn test_idempotency_cache_overflow_preserves_response_and_disables_replay() {
    let config = MiddlewareConfig {
        idempotency_ttl: Duration::from_secs(60),
        max_replay_body_size: 32,
        require_redis: false,
        ..Default::default()
    };
    let idempotency_store = Arc::new(InMemoryIdempotencyStore::new());
    let state = MiddlewareState::with_store(
        config,
        idempotency_store.clone(),
        Arc::new(InMemoryActionStore::new()),
        Arc::new(InMemoryQuotaStore::new()),
        None,
    );

    let counter = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/",
            post({
                let counter = counter.clone();
                move || {
                    let run = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    async move {
                        let mut payload = format!("run-{run}:").into_bytes();
                        payload.extend(vec![b'x'; 128]);
                        payload.extend(vec![b'y'; 128]);
                        let mut res = Response::new(Body::from(Bytes::from(payload)));
                        *res.status_mut() = StatusCode::CREATED;
                        res.headers_mut().insert(
                            "X-Action-Id",
                            HeaderValue::from_str(&format!("action-{run}"))
                                .expect("valid action id header"),
                        );
                        res
                    }
                }
            }),
        )
        .layer(middleware::from_fn(idempotency_middleware))
        .layer(Extension(state));

    let tenant_ctx = TenantContext::new(
        TenantId::new("tenant-1").expect("tenant id"),
        Some(UserId::new("user-1").expect("user id")),
    );

    let make_req = |ctx: TenantContext| {
        Request::builder()
            .method("POST")
            .uri("/")
            .header("Idempotency-Key", "cache-overflow-key")
            .extension(ctx)
            .body(Body::from("same-request-body"))
            .expect("request")
    };

    let res1 = app
        .clone()
        .oneshot(make_req(tenant_ctx.clone()))
        .await
        .unwrap();
    assert_eq!(res1.status(), StatusCode::CREATED);
    assert!(
        !res1.headers().contains_key("X-Idempotency-Replay"),
        "first response must not be replay"
    );
    let body1 = to_bytes(res1.into_body(), usize::MAX).await.unwrap();
    let body1 = String::from_utf8(body1.to_vec()).expect("utf8 body1");
    assert!(body1.starts_with("run-1:"), "body1={body1}");
    assert!(body1.len() > 32, "body must exceed replay limit");

    let scope_key = IdempotencyScopeKey {
        tenant_id: TenantId::new("tenant-1").expect("tenant id"),
        method: "POST".to_string(),
        canonical_target: "/".to_string(),
        idempotency_key: "cache-overflow-key".to_string(),
    };
    tokio::time::timeout(Duration::from_millis(500), async {
        loop {
            match idempotency_store
                .get(&scope_key)
                .await
                .expect("idempotency store get should succeed")
            {
                None => break,
                Some(IdempotencyEntry::InFlight { .. }) => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Some(IdempotencyEntry::Completed(_)) => {
                    panic!("cache overflow path must not write completed record");
                }
            }
        }
    })
    .await
    .expect("inflight lease should be released within timeout");

    let res2 = app.clone().oneshot(make_req(tenant_ctx)).await.unwrap();
    assert_eq!(res2.status(), StatusCode::CREATED);
    assert!(
        !res2.headers().contains_key("X-Idempotency-Replay"),
        "cache overflow path must not replay"
    );
    let body2 = to_bytes(res2.into_body(), usize::MAX).await.unwrap();
    let body2 = String::from_utf8(body2.to_vec()).expect("utf8 body2");
    assert!(body2.starts_with("run-2:"), "body2={body2}");
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "handler invocation count must be exactly 2"
    );
}
