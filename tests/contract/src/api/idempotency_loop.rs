use axum::{
    Extension, Router,
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::post,
};
use tower::ServiceExt; // for oneshot
use kernel_api::middleware::{MiddlewareConfig, MiddlewareState, idempotency_middleware};
use kernel_api::auth::{TenantContext, TenantId, UserId};
use std::time::Duration;
use tokio::task::JoinSet;

#[tokio::test]
async fn test_idempotency_loop_limit() {
    // Setup
    let config = MiddlewareConfig {
        idempotency_ttl: Duration::from_secs(60),
        // Large enough to avoid timeout, so we hit the loop limit instead
        inflight_wait_timeout: Duration::from_millis(1000),
        ..Default::default()
    };
    let state = MiddlewareState::new(config);

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
        service_unavailable_count + _internal_error_count + _conflict_count + _other_count,
        num_requests,
        "Missing responses from idempotency loop"
    );

    // We expect some requests to process (return 500) and some to fail acquiring (return 503).
    assert!(
        service_unavailable_count > 0,
        "Expected at least one 503 due to loop limit"
    );
}
