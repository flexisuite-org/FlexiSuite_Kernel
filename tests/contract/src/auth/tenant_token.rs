use crate::api::middleware_integration::{setup_app, setup_app_with_db};
use crate::auth::helpers::{generate_token, generate_token_with_kid, setup};
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use kernel_api::entities::{key_record, permission};
use sea_orm::{MockDatabase, MockExecResult};
use tower::ServiceExt; // for oneshot

#[tokio::test]
async fn test_tenant_token_v2_accepts_valid_token_with_kid() {
    setup();
    let now = chrono::Utc::now();
    let active_hmac_key = key_record::Model {
        kid: "hmac-test-active".to_string(),
        key_type: key_record::KeyType::Hmac,
        algorithm: "HS256".to_string(),
        secret_bytes: Some(vec![3_u8; 32]),
        public_bytes: None,
        state: key_record::KeyState::Active,
        created_at: now.into(),
        activated_at: Some(now.into()),
        retired_at: None,
        revoked_at: None,
        expires_at: None,
    };
    let db = MockDatabase::new(sea_orm::DatabaseBackend::Postgres)
        .append_exec_results(vec![MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }])
        .append_query_results(vec![vec![active_hmac_key]])
        .append_query_results(vec![Vec::<permission::Model>::new()])
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
    assert!(
        res.status() == StatusCode::UNAUTHORIZED || res.status() == StatusCode::FORBIDDEN,
        "Token without KID must be rejected (got {})",
        res.status()
    );
}
