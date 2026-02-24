use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use kernel_api::middleware::rbac::{load_permissions_middleware, require_permission};
use kernel_core::auth::{TenantContext, TenantId, UserId};
use kernel_data::entities::{permission, key_record};
use sea_orm::{
    DatabaseBackend, MockDatabase, MockExecResult,
};
use chrono::Utc;
use tower::ServiceExt;
use std::sync::Arc;
use uuid::Uuid;

#[tokio::test]
async fn test_rbac_middleware_allow() {
    let tenant_id = "tenant-1";
    let user_id = "user-1";
    let now = Utc::now();

    let perm_id = Uuid::new_v4();
    let role_id = Uuid::new_v4();

    let mock_permission = permission::Model {
        id: perm_id,
        tenant_id: tenant_id.to_string(),
        role_id,
        resource: "test".to_string(),
        action: "read".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    };

    let mock_key = key_record::Model {
        kid: "hmac-test".to_string(),
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
    };

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![mock_key.clone()]]) // KeyManager query
        .append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }]) // authorize_tenant execution
        .append_query_results([vec![mock_permission]]) // Actual permission query
        .into_connection();

    let app = axum::Router::new()
        .route("/protected", axum::routing::get(|| async { "Allowed" }).layer(axum::middleware::from_fn(|req, next| require_permission("test:read", req, next))))
        .layer(axum::middleware::from_fn(load_permissions_middleware))
        .layer(axum::Extension(TenantContext::new(
            TenantId::new(tenant_id).unwrap(),
            Some(UserId::new(user_id).unwrap()),
        ).with_db(Arc::new(db))));

    let req = Request::builder()
        .uri("/protected")
        .header("X-Tenant-Id", tenant_id)
        .header("X-User-Id", user_id)
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_rbac_middleware_deny() {
    let tenant_id = "tenant-1";
    let user_id = "user-1";
    let now = Utc::now();
 
    let mock_key = key_record::Model {
        kid: "hmac-test".to_string(),
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
    };

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![mock_key.clone()]]) // KeyManager query
        .append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }]) // authorize_tenant execution
        .append_query_results([vec![permission::Model {
            id: Uuid::new_v4(),
            tenant_id: tenant_id.to_string(),
            role_id: Uuid::new_v4(),
            resource: "other".to_string(),
            action: "write".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        }]]) // Actual permission query (denial)
        .into_connection();

    let app = axum::Router::new()
        .route("/protected", axum::routing::get(|| async { "Allowed" }).layer(axum::middleware::from_fn(|req, next| require_permission("test:read", req, next))))
        .layer(axum::middleware::from_fn(load_permissions_middleware))
        .layer(axum::Extension(TenantContext::new(
            TenantId::new(tenant_id).unwrap(),
            Some(UserId::new(user_id).unwrap()),
        ).with_db(Arc::new(db))));

    let req = Request::builder()
        .uri("/protected")
        .header("X-Tenant-Id", tenant_id)
        .header("X-User-Id", user_id)
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_rbac_middleware_anonymous() {
    let tenant_id = "tenant-1";

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([Vec::<permission::Model>::new()])
        .into_connection();

    let app = axum::Router::new()
        .route("/protected", axum::routing::get(|| async { "Allowed" }).layer(axum::middleware::from_fn(|req, next| require_permission("test:read", req, next))))
        .layer(axum::middleware::from_fn(load_permissions_middleware))
        .layer(axum::Extension(TenantContext::new(
            TenantId::new(tenant_id).unwrap(),
            None,
        ).with_db(Arc::new(db))));

    let req = Request::builder()
        .uri("/protected")
        .header("X-Tenant-Id", tenant_id)
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_rbac_middleware_db_error() {
    let tenant_id = "tenant-1";
    let user_id = "user-1";
    let now = Utc::now();

    let mock_key = key_record::Model {
        kid: "hmac-test".to_string(),
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
    };

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![mock_key.clone()]]) // KeyManager query
        .append_query_errors([sea_orm::DbErr::Custom("DB Error".to_string())])
        .into_connection();

    let app = axum::Router::new()
        .route("/protected", axum::routing::get(|| async { "Allowed" }).layer(axum::middleware::from_fn(|req, next| require_permission("test:read", req, next))))
        .layer(axum::middleware::from_fn(load_permissions_middleware))
        .layer(axum::Extension(TenantContext::new(
            TenantId::new(tenant_id).unwrap(),
            Some(UserId::new(user_id).unwrap()),
        ).with_db(Arc::new(db))));

    let req = Request::builder()
        .uri("/protected")
        .header("X-Tenant-Id", tenant_id)
        .header("X-User-Id", user_id)
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn test_rbac_middleware_missing_context() {
    let app = axum::Router::new()
        .route("/protected", axum::routing::get(|| async { "Allowed" }).layer(axum::middleware::from_fn(|req, next| require_permission("test:read", req, next))))
        .layer(axum::middleware::from_fn(load_permissions_middleware));

    let req = Request::builder()
        .uri("/protected")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
