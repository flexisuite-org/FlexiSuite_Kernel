#[cfg(feature = "dev-auth")]
use async_trait::async_trait;
#[cfg(feature = "dev-auth")]
use axum::body::to_bytes;
#[cfg(feature = "dev-auth")]
use axum::http::HeaderValue;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
#[cfg(feature = "dev-auth")]
use kernel_api::middleware::{
    IdempotencyAcquireResult, IdempotencyEntry, IdempotencyLease, IdempotencyRecord,
    IdempotencyScopeKey, IdempotencyStoreError,
};
use kernel_api::middleware::{
    IdempotencyStore, InMemoryActionStore, InMemoryQuotaStore, MiddlewareConfig, MiddlewareState,
};
use std::sync::Arc;
#[cfg(feature = "dev-auth")]
use tokio::sync::Notify;

use sea_orm::{DatabaseBackend, MockDatabase};

#[cfg(feature = "dev-auth")]
use serde_json::Value;
use tower::ServiceExt;

use chrono::Utc;
use kernel_data::entities::{key_record, permission};
use sea_orm::MockExecResult;
use uuid::Uuid;

/// Creates a mock database with the specified number of authorization budget entries.
///
/// Note: The mock permissions are hardcoded with `tenant_id = "tenant-1"`.
/// Tests using this mock MUST set both `X-Tenant-Id = tenant-1` and `X-User-Id`
/// in debug builds for protected RBAC checks to work correctly.
pub fn mock_db_with_budget(auth_calls: usize) -> sea_orm::DatabaseConnection {
    let mut db = MockDatabase::new(DatabaseBackend::Postgres);

    for _ in 0..auth_calls {
        let now = Utc::now();
        // 1) flexi.authorize_tenant
        db = db.append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }]);
        // 2) RBACRepository::get_user_permissions
        let perms = vec![
            permission::Model {
                id: Uuid::new_v4(),
                tenant_id: "tenant-1".to_string(),
                role_id: Uuid::new_v4(),
                resource: "test".to_string(),
                action: "write".to_string(),
                created_at: now.into(),
                updated_at: now.into(),
            },
            permission::Model {
                id: Uuid::new_v4(),
                tenant_id: "tenant-1".to_string(),
                role_id: Uuid::new_v4(),
                resource: "action".to_string(),
                action: "read".to_string(),
                created_at: now.into(),
                updated_at: now.into(),
            },
            permission::Model {
                id: Uuid::new_v4(),
                tenant_id: "tenant-1".to_string(),
                role_id: Uuid::new_v4(),
                resource: "diagnostics".to_string(),
                action: "read".to_string(),
                created_at: now.into(),
                updated_at: now.into(),
            },
        ];
        db = db.append_query_results([perms]);
    }

    db.into_connection()
}

pub fn mock_db_with_empty_permissions(auth_calls: usize) -> sea_orm::DatabaseConnection {
    let mut db = MockDatabase::new(DatabaseBackend::Postgres);
    for _ in 0..auth_calls {
        db = db.append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }]);
        let perms_empty = Vec::<permission::Model>::new();
        db = db.append_query_results([perms_empty]);
    }
    db.into_connection()
}

/// Companion helper for Bearer v4/public tests that trigger V4->V2 bridge path.
/// Query sequence: active HMAC key lookup -> authorize_tenant -> permissions lookup.
pub fn mock_db_with_bridge_budget(auth_calls: usize) -> sea_orm::DatabaseConnection {
    let mut db = MockDatabase::new(DatabaseBackend::Postgres);

    for _ in 0..auth_calls {
        let now = Utc::now();
        db = db.append_query_results([vec![key_record::Model {
            kid: "hmac-key-1".to_string(),
            key_type: key_record::KeyType::Hmac,
            algorithm: "HS256".to_string(),
            secret_bytes: Some(vec![0u8; 32]),
            public_bytes: None,
            state: key_record::KeyState::Active,
            created_at: now.into(),
            activated_at: Some(now.into()),
            retired_at: None,
            revoked_at: None,
            expires_at: None,
        }]]);
        db = db.append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }]);
        let perms = vec![
            permission::Model {
                id: Uuid::new_v4(),
                tenant_id: "tenant-1".to_string(),
                role_id: Uuid::new_v4(),
                resource: "test".to_string(),
                action: "write".to_string(),
                created_at: now.into(),
                updated_at: now.into(),
            },
            permission::Model {
                id: Uuid::new_v4(),
                tenant_id: "tenant-1".to_string(),
                role_id: Uuid::new_v4(),
                resource: "action".to_string(),
                action: "read".to_string(),
                created_at: now.into(),
                updated_at: now.into(),
            },
            permission::Model {
                id: Uuid::new_v4(),
                tenant_id: "tenant-1".to_string(),
                role_id: Uuid::new_v4(),
                resource: "diagnostics".to_string(),
                action: "read".to_string(),
                created_at: now.into(),
                updated_at: now.into(),
            },
        ];
        db = db.append_query_results([perms]);
    }

    db.into_connection()
}

fn default_mock_db() -> sea_orm::DatabaseConnection {
    // Default mock that allows up to 20 successful authorizations and permission checks.
    // This covers most integration tests that expect success.
    // Tests expecting failure or specific DB behavior should use setup_app_with_db.
    mock_db_with_budget(20)
}

pub async fn setup_app() -> axum::Router {
    setup_app_with_db(default_mock_db()).await
}

pub async fn setup_app_with_db(db: sea_orm::DatabaseConnection) -> axum::Router {
    setup_app_with_config_and_db(MiddlewareConfig::default(), None, db).await
}

pub async fn setup_app_with_config(
    config: MiddlewareConfig,
    store: Option<Arc<dyn IdempotencyStore>>,
) -> axum::Router {
    setup_app_with_config_and_db(config, store, default_mock_db()).await
}

pub async fn setup_app_with_config_and_db(
    mut config: MiddlewareConfig,
    store: Option<Arc<dyn IdempotencyStore>>,
    db: sea_orm::DatabaseConnection,
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

    let (app, _cleanup) = kernel_api::build_app_with_state(state, db.into());
    app
}

#[cfg(feature = "dev-auth")]
struct NotifyingStore {
    inner: Arc<dyn IdempotencyStore>,
    notify: Arc<Notify>,
}

#[cfg(feature = "dev-auth")]
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

    #[cfg(feature = "dev-auth")]
    {
        builder = builder.header("X-Tenant-Id", "tenant-1");
        builder = builder.header("X-User-Id", "user-1");
    }

    #[cfg(not(feature = "dev-auth"))]
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

    #[cfg(feature = "dev-auth")]
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
            .header("X-Tenant-Id", "tenant-1") // matched with mock db
            .header("X-User-Id", "user-1")
            .header("Idempotency-Key", "auth-test-key")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
    }
}

#[tokio::test]
async fn test_rbac_fail_closed_with_empty_permissions_fixture() {
    let app = setup_app_with_db(mock_db_with_empty_permissions(1)).await;

    let mut builder = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Idempotency-Key", "rbac-empty-perms-key");

    #[cfg(feature = "dev-auth")]
    {
        builder = builder.header("X-Tenant-Id", "tenant-1");
        builder = builder.header("X-User-Id", "user-1");
    }

    #[cfg(not(feature = "dev-auth"))]
    {
        builder = builder.header("Authorization", "Bearer invalid");
    }

    let req = builder.body(Body::empty()).unwrap();
    let res = app.clone().oneshot(req).await.unwrap();

    #[cfg(feature = "dev-auth")]
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    #[cfg(not(feature = "dev-auth"))]
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[cfg(feature = "dev-auth")]
async fn test_dev_auth_uses_dev_token_for_db_authorization() {
    let app = setup_app_with_db(mock_db_with_budget(1)).await;

    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .header("X-Tenant-Id", "tenant-1")
        .header("X-User-Id", "user-1")
        .header("Idempotency-Key", "dev-auth-db-token")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
}

#[tokio::test]
#[cfg(feature = "dev-auth")]
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
    #[cfg(feature = "dev-auth")]
    {
        builder = builder.header("X-Tenant-Id", "tenant-1");
        builder = builder.header("X-User-Id", "user-1");
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
#[cfg(not(feature = "dev-auth"))]
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

    #[cfg(feature = "dev-auth")]
    {
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }

    #[cfg(not(feature = "dev-auth"))]
    {
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn test_quota_evaluation_priority_and_clipping() {
    let app = setup_app().await;

    let mut builder = Request::builder().uri("/test").method("POST");
    #[cfg(feature = "dev-auth")]
    {
        builder = builder.header("X-Tenant-Id", "tenant-1");
        builder = builder.header("X-User-Id", "user-1");
    }
    #[cfg(not(feature = "dev-auth"))]
    {
        builder = builder.header("Authorization", "Bearer invalid");
    }

    // Use a unique idempotency key to ensure this test always triggers fresh quota evaluation.
    // The key includes the feature flag to ensure isolation between test-utils and non-test-utils runs.
    #[cfg(feature = "test-utils")]
    let idempotency_key = "quota-test-key-test-utils";
    #[cfg(not(feature = "test-utils"))]
    let idempotency_key = "quota-test-key-no-test-utils";

    let req = builder
        .header("X-Mock-Quota-System", "true")
        .header("X-Mock-Quota-Tenant", "true")
        .header("Idempotency-Key", idempotency_key)
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();

    // When test-utils is enabled, the mock quota middleware returns 503 SERVICE_UNAVAILABLE
    // because the X-Mock-Quota-System header triggers a simulated quota violation.
    // Without test-utils, the mock quota headers are ignored and the request succeeds with 201.
    #[cfg(all(feature = "dev-auth", feature = "test-utils"))]
    {
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let retry_after = res.headers().get("Retry-After").unwrap().to_str().unwrap();
        assert_eq!(retry_after, "30");
    }

    #[cfg(all(feature = "dev-auth", not(feature = "test-utils")))]
    {
        assert_eq!(res.status(), StatusCode::CREATED);
    }

    #[cfg(not(feature = "dev-auth"))]
    {
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
#[cfg(feature = "dev-auth")]
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
            tokio::time::timeout(std::time::Duration::from_secs(5), notify.notified())
                .await
                .expect("timed out waiting for t1 to acquire idempotency lock");
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
