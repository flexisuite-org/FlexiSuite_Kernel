use crate::api::middleware_integration::setup_app_with_db;
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
async fn test_key_revocation_slo() {
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

    // Mock DB that expects one successful authorization (for Case 1)
    // Case 2 and 3 should be rejected by Auth middleware (stateless/cached) and not hit DB.
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
