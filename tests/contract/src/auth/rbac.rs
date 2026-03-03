use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use kernel_api::auth::{TenantContext, TenantId, UserId};
use kernel_api::middleware::{BearerToken, load_permissions_middleware, require_permission};
use kernel_data::entities::permission;
use sea_orm::{DatabaseBackend, MockDatabase};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// Note: Full integration test with Testcontainers (real Postgres)
// verifying the SQL join query is implemented in `kernel-data/tests/integration_tests.rs`
// (test_rbac_integration_real_postgres).
// These tests verify the middleware logic using MockDatabase.

#[tokio::test]
async fn test_rbac_middleware_allow() {
    let tenant_id = "tenant-1";
    let user_id = "user-1";
    let token = "v2:kid:ts:nonce:tenant-1:sig"; // Mock token format matching parser expectation

    let perm_id = Uuid::new_v4();
    let role_id = Uuid::new_v4();
    let now = chrono::Utc::now();

    // 1. First query: authorize_tenant (called by with_tenant_tx)
    // 2. Second query: RBACRepository::get_user_permissions
    // 3. Third query: commit (implicit in with_tenant_tx logic for successful runs?) - Actually with_tenant_tx does commit.

    // We mock the authorize_tenant call to succeed
    // Note: MockDatabase::append_exec_results is for execute, append_query_results is for select.
    // authorize_tenant is called via execute("SELECT flexi.authorize_tenant(...)")
    // MockDatabase handles execute by returning ExecResult, not QueryResult.

    let mock_permission = permission::Model {
        id: perm_id,
        tenant_id: tenant_id.to_string(),
        role_id,
        resource: "test".to_string(),
        action: "read".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    };

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_exec_results(vec![sea_orm::MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }]) // authorize_tenant (execute)
        .append_query_results(vec![vec![mock_permission]]) // permissions query (select)
        .into_connection();

    let app = axum::Router::new()
        .route(
            "/protected",
            axum::routing::get(|| async { "Allowed" }).layer(axum::middleware::from_fn(
                |req, next| require_permission("test:read", req, next),
            )),
        )
        .layer(axum::middleware::from_fn(load_permissions_middleware))
        .layer(axum::Extension(BearerToken::new(token.to_string())))
        .layer(axum::Extension(
            TenantContext::new(
                TenantId::new(tenant_id).unwrap(),
                Some(UserId::new(user_id).unwrap()),
            )
            .with_db(Arc::new(db)),
        ));

    let req = Request::builder()
        .uri("/protected")
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_rbac_middleware_deny() {
    let tenant_id = "tenant-1";
    let user_id = "user-1";
    let token = "v2:kid:ts:nonce:tenant-1:sig";

    // User has other:write, but needs test:read
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_exec_results(vec![sea_orm::MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }]) // authorize_tenant (execute)
        .append_query_results(vec![vec![permission::Model {
            id: Uuid::new_v4(),
            tenant_id: tenant_id.to_string(),
            role_id: Uuid::new_v4(),
            resource: "other".to_string(),
            action: "write".to_string(),
            created_at: chrono::Utc::now().into(),
            updated_at: chrono::Utc::now().into(),
        }]])
        .into_connection();

    let app = axum::Router::new()
        .route(
            "/protected",
            axum::routing::get(|| async { "Allowed" }).layer(axum::middleware::from_fn(
                |req, next| require_permission("test:read", req, next),
            )),
        )
        .layer(axum::middleware::from_fn(load_permissions_middleware))
        .layer(axum::Extension(BearerToken::new(token.to_string())))
        .layer(axum::Extension(
            TenantContext::new(
                TenantId::new(tenant_id).unwrap(),
                Some(UserId::new(user_id).unwrap()),
            )
            .with_db(Arc::new(db)),
        ));

    let req = Request::builder()
        .uri("/protected")
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_rbac_middleware_503_on_no_active_key() {
    let tenant_id = "tenant-1";
    let user_id = "user-1";
    let token = "v4.public.something"; // V4 token triggers KeyManager::generate_tenant_token

    // Mock DB returns None for find active key query
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([
            Vec::<kernel_data::entities::key_record::Model>::new(), // get_active_key_internal returns None -> NoActiveKey error
        ])
        .into_connection();

    let app = axum::Router::new()
        .route("/protected", axum::routing::get(|| async { "Allowed" }))
        .layer(axum::middleware::from_fn(load_permissions_middleware))
        .layer(axum::Extension(BearerToken::new(token.to_string())))
        .layer(axum::Extension(
            TenantContext::new(
                TenantId::new(tenant_id).unwrap(),
                Some(UserId::new(user_id).unwrap()),
            )
            .with_db(Arc::new(db)),
        ));

    let req = Request::builder()
        .uri("/protected")
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn test_rbac_rejects_v2_token_without_kid() {
    // REQ-TENANT-TOKEN-V2: Tokens without kid MUST be rejected
    let tenant_id = "tenant-1";
    let user_id = "user-1";
    // Malformed v2 token with empty kid field
    let token_no_kid = "v2::1234567890:nonce:tenant-1:sig";

    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    let app = axum::Router::new()
        .route("/protected", axum::routing::get(|| async { "Allowed" }))
        .layer(axum::middleware::from_fn(load_permissions_middleware))
        .layer(axum::Extension(BearerToken::new(token_no_kid.to_string())))
        .layer(axum::Extension(
            TenantContext::new(
                TenantId::new(tenant_id).unwrap(),
                Some(UserId::new(user_id).unwrap()),
            )
            .with_db(Arc::new(db)),
        ));

    let req = Request::builder()
        .uri("/protected")
        .header("Authorization", format!("Bearer {}", token_no_kid))
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "Token without kid must be rejected (REQ-TENANT-TOKEN-V2)"
    );
}

#[tokio::test]
async fn test_rbac_rejects_v2_token_with_insufficient_parts() {
    // REQ-TENANT-TOKEN-V2: Malformed v2 tokens MUST be rejected
    let tenant_id = "tenant-1";
    let user_id = "user-1";
    // Invalid v2 token with only 3 parts
    let token_malformed = "v2:kid:timestamp";

    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    let app = axum::Router::new()
        .route("/protected", axum::routing::get(|| async { "Allowed" }))
        .layer(axum::middleware::from_fn(load_permissions_middleware))
        .layer(axum::Extension(BearerToken::new(token_malformed.to_string())))
        .layer(axum::Extension(
            TenantContext::new(
                TenantId::new(tenant_id).unwrap(),
                Some(UserId::new(user_id).unwrap()),
            )
            .with_db(Arc::new(db)),
        ));

    let req = Request::builder()
        .uri("/protected")
        .header("Authorization", format!("Bearer {}", token_malformed))
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "Malformed v2 token must be rejected"
    );
}

#[tokio::test]
async fn test_rbac_accepts_v2_token_with_valid_kid() {
    // REQ-TENANT-TOKEN-V2: Tokens with valid kid MUST be accepted
    let tenant_id = "tenant-1";
    let user_id = "user-1";
    let token_with_kid = "v2:hmac-key-1:1234567890:uuid-nonce:tenant-1:abcdef1234";

    let mock_permission = permission::Model {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        role_id: Uuid::new_v4(),
        resource: "test".to_string(),
        action: "read".to_string(),
        created_at: chrono::Utc::now().into(),
        updated_at: chrono::Utc::now().into(),
    };

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_exec_results(vec![sea_orm::MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }]) // authorize_tenant
        .append_query_results(vec![vec![mock_permission]]) // permissions query
        .into_connection();

    let app = axum::Router::new()
        .route(
            "/protected",
            axum::routing::get(|| async { "Allowed" }).layer(axum::middleware::from_fn(
                |req, next| require_permission("test:read", req, next),
            )),
        )
        .layer(axum::middleware::from_fn(load_permissions_middleware))
        .layer(axum::Extension(BearerToken::new(token_with_kid.to_string())))
        .layer(axum::Extension(
            TenantContext::new(
                TenantId::new(tenant_id).unwrap(),
                Some(UserId::new(user_id).unwrap()),
            )
            .with_db(Arc::new(db)),
        ));

    let req = Request::builder()
        .uri("/protected")
        .header("Authorization", format!("Bearer {}", token_with_kid))
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::OK,
        "Token with valid kid must be accepted (REQ-TENANT-TOKEN-V2)"
    );
}

#[tokio::test]
async fn test_rbac_fails_without_tenant_context() {
    // Middleware ordering enforcement: load_permissions_middleware MUST fail without TenantContext
    let token = "v2:kid:ts:nonce:tenant-1:sig";

    let _db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    let app = axum::Router::new()
        .route("/protected", axum::routing::get(|| async { "Allowed" }))
        .layer(axum::middleware::from_fn(load_permissions_middleware))
        // Missing TenantContext extension - simulates skipped authentication
        .layer(axum::Extension(BearerToken::new(token.to_string())));

    let req = Request::builder()
        .uri("/protected")
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "RBAC middleware must fail without TenantContext (authentication bypass protection)"
    );
}

#[tokio::test]
async fn test_rbac_fails_without_bearer_token() {
    // Middleware ordering enforcement: load_permissions_middleware MUST fail without BearerToken
    let tenant_id = "tenant-1";
    let user_id = "user-1";

    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    let app = axum::Router::new()
        .route("/protected", axum::routing::get(|| async { "Allowed" }))
        .layer(axum::middleware::from_fn(load_permissions_middleware))
        // Missing BearerToken extension - simulates skipped authentication
        .layer(axum::Extension(
            TenantContext::new(
                TenantId::new(tenant_id).unwrap(),
                Some(UserId::new(user_id).unwrap()),
            )
            .with_db(Arc::new(db)),
        ));

    let req = Request::builder()
        .uri("/protected")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::FORBIDDEN,
        "RBAC middleware must fail without BearerToken (authentication bypass protection)"
    );
}
