use async_trait::async_trait;
#[cfg(debug_assertions)]
use axum::body::to_bytes;
#[cfg(debug_assertions)]
use axum::http::HeaderValue;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use kernel_api::entities::{key_record, permission};
use kernel_api::middleware::{
    IdempotencyAcquireResult, IdempotencyEntry, IdempotencyLease, IdempotencyRecord,
    IdempotencyScopeKey, IdempotencyStore, IdempotencyStoreError, InMemoryActionStore,
    InMemoryQuotaStore, MiddlewareConfig, MiddlewareState,
};
use migration::MigratorTrait;
use sea_orm::{ActiveModelTrait, ConnectionTrait, Database, DatabaseConnection, Set, Statement};
use std::sync::Arc;
use std::sync::OnceLock;
use testcontainers::{RunnableImage, clients};
use testcontainers_modules::postgres::Postgres;
use tokio::sync::Notify;

use sea_orm::{DatabaseBackend, MockDatabase};

#[cfg(debug_assertions)]
use serde_json::Value;
use tower::ServiceExt;

use sea_orm::MockExecResult;

fn default_mock_db() -> sea_orm::DatabaseConnection {
    // Default mock that allows up to 20 successful authorizations and permission checks.
    // This covers most integration tests that expect success.
    // Tests expecting failure or specific DB behavior should use setup_app_with_db.
    let mut db = MockDatabase::new(DatabaseBackend::Postgres);

    for i in 0..20 {
        let now = chrono::Utc::now();
        let active_hmac_key = key_record::Model {
            kid: format!("hmac-test-active-{i}"),
            key_type: key_record::KeyType::Hmac,
            algorithm: "HS256".to_string(),
            secret_bytes: Some(vec![1_u8; 32]),
            public_bytes: None,
            state: key_record::KeyState::Active,
            created_at: now.into(),
            activated_at: Some(now.into()),
            retired_at: None,
            revoked_at: None,
            expires_at: None,
        };

        db = db
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }]) // authorize_tenant
            .append_query_results(vec![vec![active_hmac_key]]) // active key query for server-minted tenant token
            .append_query_results(vec![Vec::<permission::Model>::new()]); // get_user_permissions (empty list)
    }

    db.into_connection()
}

type PostgresNode = testcontainers::Container<'static, Postgres>;

fn get_docker_client() -> &'static clients::Cli {
    static DOCKER: OnceLock<&'static clients::Cli> = OnceLock::new();
    DOCKER.get_or_init(|| Box::leak(Box::new(clients::Cli::default())))
}

pub async fn setup_real_postgres_db() -> (DatabaseConnection, PostgresNode) {
    const TEST_INTERNAL_SECRET: &str = "contract_test_internal_secret_123";

    let docker = get_docker_client();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to connect to Postgres container");

    db.execute(Statement::from_string(
        sea_orm::DbBackend::Postgres,
        "DO $$ BEGIN CREATE ROLE flexi NOLOGIN; EXCEPTION WHEN duplicate_object THEN NULL; END $$;"
            .to_string(),
    ))
    .await
    .expect("Failed to ensure flexi role");

    migration::Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");

    db.execute(Statement::from_string(
        sea_orm::DbBackend::Postgres,
        format!(
            "ALTER ROLE postgres SET flexi.hmac_secret = '{}'",
            TEST_INTERNAL_SECRET
        ),
    ))
    .await
    .expect("Failed to set flexi.hmac_secret");

    drop(db);
    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to reconnect to Postgres container");

    let now = chrono::Utc::now();
    key_record::ActiveModel {
        kid: Set("hmac-contract-active".to_string()),
        key_type: Set(key_record::KeyType::Hmac),
        algorithm: Set("HS256".to_string()),
        secret_bytes: Set(Some(vec![42_u8; 32])),
        public_bytes: Set(None),
        state: Set(key_record::KeyState::Active),
        created_at: Set(now.into()),
        activated_at: Set(Some(now.into())),
        retired_at: Set(None),
        revoked_at: Set(None),
        expires_at: Set(None),
    }
    .insert(&db)
    .await
    .expect("Failed to seed active HMAC key");

    (db, node)
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

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires Docker (Testcontainers)"]
async fn test_rbac_rls_real_postgres_contract() {
    use kernel_api::entities::{group, group_member, group_role, role};
    use uuid::Uuid;

    let (db, _node) = setup_real_postgres_db().await;

    let tenant_a = "tenant-a";
    let tenant_b = "tenant-b";
    let user_a = "user-a";
    let now = chrono::Utc::now();
    let role_id = Uuid::now_v7();
    let group_id = Uuid::now_v7();

    role::ActiveModel {
        id: Set(role_id),
        tenant_id: Set(tenant_a.to_string()),
        name: Set("reader".to_string()),
        description: Set("reader role".to_string()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(&db)
    .await
    .expect("Failed to insert role");

    group::ActiveModel {
        id: Set(group_id),
        tenant_id: Set(tenant_a.to_string()),
        name: Set("group-a".to_string()),
        description: Set("group a".to_string()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(&db)
    .await
    .expect("Failed to insert group");

    group_role::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_a.to_string()),
        group_id: Set(group_id),
        role_id: Set(role_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(&db)
    .await
    .expect("Failed to insert group_role");

    group_member::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_a.to_string()),
        group_id: Set(group_id),
        user_id: Set(user_a.to_string()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(&db)
    .await
    .expect("Failed to insert group_member");

    permission::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_a.to_string()),
        role_id: Set(role_id),
        resource: Set("test".to_string()),
        action: Set("read".to_string()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(&db)
    .await
    .expect("Failed to insert permission");

    let app = setup_app_with_db(db).await;

    let allow_req = Request::builder()
        .uri("/test/protected")
        .method("GET")
        .header("X-Tenant-Id", tenant_a)
        .header("X-User-Id", user_a)
        .body(Body::empty())
        .unwrap();
    let allow_res = app.clone().oneshot(allow_req).await.unwrap();
    assert_eq!(
        allow_res.status(),
        StatusCode::OK,
        "tenant A user must be authorized via real RBAC join + RLS path"
    );

    let deny_req = Request::builder()
        .uri("/test/protected")
        .method("GET")
        .header("X-Tenant-Id", tenant_b)
        .header("X-User-Id", user_a)
        .body(Body::empty())
        .unwrap();
    let deny_res = app.clone().oneshot(deny_req).await.unwrap();
    assert_eq!(
        deny_res.status(),
        StatusCode::FORBIDDEN,
        "tenant B must not see tenant A permissions via RLS-scoped RBAC query"
    );
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
        builder = builder
            .header("X-Tenant-Id", "tenant-1")
            .header("X-User-Id", "user-1");
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
            .header("X-User-Id", "user-dev")
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
        builder = builder
            .header("X-Tenant-Id", "tenant-1")
            .header("X-User-Id", "user-1");
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
        builder = builder
            .header("X-Tenant-Id", "tenant-1")
            .header("X-User-Id", "user-1");
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
