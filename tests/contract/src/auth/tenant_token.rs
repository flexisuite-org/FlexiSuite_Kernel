use crate::api::middleware_integration::{setup_app, setup_app_with_db};
use crate::auth::helpers::{generate_token, generate_token_with_kid, setup};
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use sea_orm::{MockDatabase, MockExecResult};
use tower::ServiceExt; // for oneshot
use kernel_data::entities::permission;
use uuid::Uuid;
use chrono::Utc;

#[tokio::test]
async fn test_tenant_token_v2_accepts_valid_token_with_kid() {
    setup();

    let now = Utc::now();
    let perm = permission::Model {
        id: Uuid::new_v4(),
        tenant_id: "tenant_001".to_string(),
        role_id: Uuid::new_v4(),
        resource: "test".to_string(),
        action: "write".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    };

    let db = MockDatabase::new(sea_orm::DatabaseBackend::Postgres)
        .append_exec_results(vec![
            MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            },
        ])
        .append_query_results(vec![vec![perm]])
        .into_connection();
    let app = setup_app_with_db(db).await;

    let token = generate_token(true);
    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Authorization", format!("Bearer {}", token))
        .header("Idempotency-Key", "tenant-token-v2-with-kid")
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::CREATED,
        "Token with KID must be accepted"
    );
}

#[tokio::test]
async fn test_tenant_token_v2_kid_required_contract() {
    setup();
    let app = setup_app().await;
    let token_no_kid = generate_token_with_kid(true, None);
    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Authorization", format!("Bearer {}", token_no_kid))
        .header("Idempotency-Key", "tenant-token-v2-required")
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "Token without KID must be rejected (got {})",
        res.status()
    );
}
