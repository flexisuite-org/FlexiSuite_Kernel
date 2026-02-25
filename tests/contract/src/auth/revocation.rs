use crate::api::middleware_integration::setup_app_with_db;
use crate::auth::helpers::{generate_token, generate_token_with_kid, setup};
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use kernel_api::entities::{key_record, permission};
use sea_orm::{MockDatabase, MockExecResult};
use tower::ServiceExt; // for oneshot

#[tokio::test]
async fn test_key_revocation_slo() {
    setup();
    let now = chrono::Utc::now();
    let active_hmac_key = key_record::Model {
        kid: "hmac-test-active".to_string(),
        key_type: key_record::KeyType::Hmac,
        algorithm: "HS256".to_string(),
        secret_bytes: Some(vec![5_u8; 32]),
        public_bytes: None,
        state: key_record::KeyState::Active,
        created_at: now.into(),
        activated_at: Some(now.into()),
        retired_at: None,
        revoked_at: None,
        expires_at: None,
    };

    // Mock DB that expects one successful authorization (for Case 1)
    // Case 2 and 3 should be rejected by Auth middleware (stateless/cached) and not hit DB.
    let db = MockDatabase::new(sea_orm::DatabaseBackend::Postgres)
        .append_exec_results(vec![MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }])
        .append_query_results(vec![vec![active_hmac_key]])
        .append_query_results(vec![Vec::<permission::Model>::new()])
        .into_connection();

    let app = setup_app_with_db(db).await;

    // REQ-KEY-REVOCATION-SLO: Revoked key must be rejected.

    // Case 1: Active Key -> OK
    let token_active = generate_token(true);
    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Authorization", format!("Bearer {}", token_active))
        .header("Idempotency-Key", "rev-key-1")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();

    // Loud assertion for active key
    assert_eq!(
        res.status(),
        StatusCode::CREATED,
        "Valid Active Key token rejected: status {}",
        res.status()
    );

    // Case 2: Revoked Key -> FAIL
    let token_revoked = generate_token_with_kid(true, Some("revoked"));
    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Authorization", format!("Bearer {}", token_revoked))
        .header("Idempotency-Key", "rev-key-2")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();

    // Strict assertion for contract suite
    assert!(
        res.status() == StatusCode::UNAUTHORIZED || res.status() == StatusCode::FORBIDDEN,
        "Revoked key must be rejected (got {})",
        res.status()
    );

    // Case 3: Legacy (no kid) while revoked_kids configured -> FAIL
    let token_legacy = generate_token_with_kid(true, None);
    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Authorization", format!("Bearer {}", token_legacy))
        .header("Idempotency-Key", "rev-key-3")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::UNAUTHORIZED,
        "Legacy token must be rejected when revoked_kids are configured (got {})",
        res.status()
    );
}
