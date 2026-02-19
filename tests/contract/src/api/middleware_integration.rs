use async_trait::async_trait;
#[cfg(debug_assertions)]
use axum::body::to_bytes;
#[cfg(debug_assertions)]
use axum::http::HeaderValue;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use kernel_api::middleware::{
    IdempotencyAcquireResult, IdempotencyEntry, IdempotencyLease, IdempotencyRecord,
    IdempotencyScopeKey, IdempotencyStore, IdempotencyStoreError, InMemoryActionStore,
    InMemoryQuotaStore, MiddlewareConfig, MiddlewareState,
};
use std::sync::Arc;
use tokio::sync::Notify;

use sea_orm::{DatabaseBackend, MockDatabase};

#[cfg(debug_assertions)]
use serde_json::Value;
use tower::ServiceExt;

pub async fn setup_app() -> axum::Router {
    setup_app_with_config(MiddlewareConfig::default(), None).await
}

pub async fn setup_app_with_config(
    mut config: MiddlewareConfig,
    store: Option<Arc<dyn IdempotencyStore>>,
) -> axum::Router {
    config.require_redis = false;
    let state = if let Some(s) = store {
        MiddlewareState::with_store(
            config,
            s,
            Arc::new(InMemoryActionStore::new()),
            Arc::new(InMemoryQuotaStore::new()),
        )
    } else {
        MiddlewareState::new(config)
            .await
            .expect("middleware state")
    };

    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    let (app, _cleanup) = kernel_api::build_app_with_state(state, db.into());
    app
}

#[cfg(debug_assertions)]
struct NotifyingStore {
    inner: Arc<dyn IdempotencyStore>,
    notify: Arc<Notify>,
}

#[cfg(debug_assertions)]
#[async_trait]
impl IdempotencyStore for NotifyingStore {
    async fn get(
        &self,
        key: &IdempotencyScopeKey,
    ) -> Result<Option<IdempotencyEntry>, IdempotencyStoreError> {
        self.inner.get(key).await
    }
    async fn try_acquire(
        &self,
        key: IdempotencyScopeKey,
        hash: String,
        ttl: std::time::Duration,
    ) -> Result<IdempotencyAcquireResult, IdempotencyStoreError> {
        let res = self.inner.try_acquire(key, hash, ttl).await;
        if matches!(res, Ok(IdempotencyAcquireResult::Acquired(_))) {
            self.notify.notify_one();
        }
        res
    }
    async fn complete(
        &self,
        key: IdempotencyScopeKey,
        lease: &IdempotencyLease,
        record: IdempotencyRecord,
    ) -> Result<(), IdempotencyStoreError> {
        self.inner.complete(key, lease, record).await
    }
    async fn release_inflight(
        &self,
        key: &IdempotencyScopeKey,
        lease: &IdempotencyLease,
    ) -> Result<(), IdempotencyStoreError> {
        self.inner.release_inflight(key, lease).await
    }
    async fn cleanup(&self) {
        self.inner.cleanup().await
    }
}

fn build_idempotent_post(key: &str, body: &str) -> Request<Body> {
    let mut builder = Request::builder().method("POST").uri("/test");

    #[cfg(debug_assertions)]
    {
        builder = builder.header("X-Tenant-Id", "tenant-1");
    }

    #[cfg(not(debug_assertions))]
    {
        builder = builder.header("Authorization", "Bearer invalid");
    }

    builder
        .header("Idempotency-Key", key)
        .body(Body::from(body.to_owned()))
        .unwrap()
}

#[tokio::test]
async fn test_health_is_public() {
    let app = setup_app().await;

    let req = Request::builder()
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_auth_logic_401_403() {
    let app = setup_app().await;

    // 1. Missing auth context on protected endpoint -> 401
    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 2. Invalid bearer format -> 401
    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Authorization", "Invalid token")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    #[cfg(debug_assertions)]
    {
        // 3. Malformed dev tenant header -> 403
        let req = Request::builder()
            .uri("/test")
            .method("POST")
            .header(
                "X-Tenant-Id",
                HeaderValue::from_bytes(&[0xff]).expect("header value"),
            )
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // 4. Valid dev tenant header -> 201
        let req = Request::builder()
            .uri("/test")
            .method("POST")
            .header("X-Tenant-Id", "tenant-dev")
            .header("Idempotency-Key", "auth-test-key")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
    }
}

#[tokio::test]
#[cfg(debug_assertions)]
async fn test_idempotency_conflict_scope_and_action_lookup() {
    let app = setup_app().await;

    // 1. Success first call
    let req = build_idempotent_post("key-1", "payload-a");
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let first_action_id = res
        .headers()
        .get("X-Action-Id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let first_body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let first_json: Value = serde_json::from_slice(&first_body).unwrap();
    assert_eq!(first_json["action_id"], first_action_id);

    // 2. Replay with same body -> same action_id + replay header
    let req = build_idempotent_post("key-1", "payload-a");
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(
        res.headers().get("X-Action-Id").unwrap(),
        first_action_id.as_str()
    );
    assert_eq!(res.headers().get("X-Idempotency-Replay").unwrap(), "true");
    let replay_body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let replay_json: Value = serde_json::from_slice(&replay_body).unwrap();
    assert_eq!(replay_json["action_id"], first_action_id);

    // 3. Conflict with different body -> 409
    let req = build_idempotent_post("key-1", "payload-b");
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);

    // 4. Action lookup contract
    let mut builder = Request::builder()
        .uri(format!("/actions/{first_action_id}"))
        .method("GET");
    #[cfg(debug_assertions)]
    {
        builder = builder.header("X-Tenant-Id", "tenant-1");
    }
    let req = builder.body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["action_id"], first_action_id);
    assert_eq!(json["status"], "COMPLETED");
}

#[tokio::test]
#[cfg(not(debug_assertions))]
async fn test_idempotency_conflict_scope_and_action_lookup() {
    let app = setup_app().await;
    let req = build_idempotent_post("key-1", "payload-a");
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_idempotency_key_validation() {
    let app = setup_app().await;

    let too_long = "k".repeat(129);
    let req = build_idempotent_post(&too_long, "payload");
    let res = app.clone().oneshot(req).await.unwrap();

    #[cfg(debug_assertions)]
    {
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[cfg(not(debug_assertions))]
    {
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn test_quota_evaluation_priority_and_clipping() {
    let app = setup_app().await;

    let mut builder = Request::builder().uri("/test").method("POST");
    #[cfg(debug_assertions)]
    {
        builder = builder.header("X-Tenant-Id", "tenant-1");
    }
    #[cfg(not(debug_assertions))]
    {
        builder = builder.header("Authorization", "Bearer invalid");
    }

    let req = builder
        .header("X-Mock-Quota-System", "true")
        .header("X-Mock-Quota-Tenant", "true")
        .header("Idempotency-Key", "quota-test-key")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();

    #[cfg(debug_assertions)]
    {
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let retry_after = res.headers().get("Retry-After").unwrap().to_str().unwrap();
        assert_eq!(retry_after, "30");
    }

    #[cfg(not(debug_assertions))]
    {
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
#[cfg(debug_assertions)]
async fn test_idempotency_inflight_concurrency() {
    let notify = Arc::new(Notify::new());
    let store = Arc::new(NotifyingStore {
        inner: Arc::new(kernel_api::middleware::InMemoryIdempotencyStore::new()),
        notify: notify.clone(),
    });
    let app = setup_app_with_config(MiddlewareConfig::default(), Some(store)).await;

    let t1 = tokio::spawn({
        let app = app.clone();
        async move {
            let req = build_idempotent_post("key-concurrent", "payload-c");
            app.oneshot(req).await.unwrap()
        }
    });

    let t2 = tokio::spawn({
        let app = app.clone();
        let notify = notify.clone();
        async move {
            // Wait until t1 has entered the middleware and acquired the in-flight lock
            notify.notified().await;
            let req = build_idempotent_post("key-concurrent", "payload-c");
            app.oneshot(req).await.unwrap()
        }
    });

    let (res1, res2) = tokio::join!(t1, t2);
    let res1 = res1.unwrap();
    let res2 = res2.unwrap();

    assert_eq!(res1.status(), StatusCode::CREATED);
    assert_eq!(res2.status(), StatusCode::CREATED);

    let id1 = res1
        .headers()
        .get("X-Action-Id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let id2 = res2
        .headers()
        .get("X-Action-Id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(id1, id2);

    let replay_count = (if res1.headers().contains_key("X-Idempotency-Replay") {
        1
    } else {
        0
    }) + (if res2.headers().contains_key("X-Idempotency-Replay") {
        1
    } else {
        0
    });

    assert_eq!(replay_count, 1, "Exactly one request should be a replay");
}
