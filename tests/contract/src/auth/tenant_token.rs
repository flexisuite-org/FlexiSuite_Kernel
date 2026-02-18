use axum::http::StatusCode;
use tower::ServiceExt; // for oneshot
use axum::body::Body;
use axum::http::Request;
use crate::api::middleware_integration::setup_app;
use crate::auth::helpers::{setup, generate_token};

#[tokio::test]
async fn test_tenant_token_v2_kid_required() {
    setup();
    let app = setup_app().await;

    // Case 1: Token with KID (should pass)
    let token_with_kid = generate_token(Some("key-1"), true);
    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Authorization", format!("Bearer {}", token_with_kid))
        .header("Idempotency-Key", "test-key-1")
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();

    assert_eq!(res.status(), StatusCode::CREATED, "Valid token with KID should be accepted");

    // Case 2: Token WITHOUT KID (should fail)
    let token_no_kid = generate_token(None, true);
    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Authorization", format!("Bearer {}", token_no_kid))
        .header("Idempotency-Key", "test-key-2")
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();

    // Strict assertion
    assert!(
        res.status() == StatusCode::UNAUTHORIZED || res.status() == StatusCode::FORBIDDEN,
        "Token without KID must be rejected (got {})", res.status()
    );
}
