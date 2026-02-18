use crate::api::middleware_integration::setup_app;
use crate::auth::helpers::{generate_token, generate_token_with_kid, setup};
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt; // for oneshot

#[tokio::test]
async fn test_key_revocation_slo() {
    setup();
    // Use public setup_app
    let app = setup_app().await;

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
}
