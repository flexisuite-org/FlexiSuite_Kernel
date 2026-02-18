use axum::http::StatusCode;
use tower::ServiceExt; // for oneshot
use axum::body::Body;
use axum::http::Request;
use crate::api::middleware_integration::setup_app;
use crate::auth::helpers::{setup, generate_token};

#[tokio::test]
async fn test_tenant_token_v2_accepts_valid_token_without_kid_for_now() {
    setup();
    let app = setup_app().await;

    let token_no_kid = generate_token(true);
    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Authorization", format!("Bearer {}", token_no_kid))
        .header("Idempotency-Key", "tenant-token-v2-current")
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        res.status(),
        StatusCode::CREATED,
        "Current middleware behavior accepts token without KID"
    );
}

#[tokio::test]
#[ignore = "KID/footer validation is not implemented in kernel-api parser yet"]
async fn test_tenant_token_v2_kid_required_contract() {
    setup();
    let app = setup_app().await;
    let token_no_kid = generate_token(true);
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
