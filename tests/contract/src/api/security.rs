use crate::api::middleware_integration::setup_app;
#[cfg(not(debug_assertions))]
use crate::auth::helpers::generate_token;
use crate::auth::helpers::{generate_token_with_claims, setup};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::ServiceExt;

#[tokio::test]
async fn test_security_headers_present_on_ok_201() {
    setup();
    let app = setup_app().await;

    let mut builder = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Idempotency-Key", "sec-test-201");
    #[cfg(debug_assertions)]
    {
        builder = builder.header("X-Tenant-Id", "tenant-1");
        builder = builder.header("X-User-Id", "user-1");
    }
    #[cfg(not(debug_assertions))]
    {
        let token = generate_token(true);
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
    let req = builder.body(Body::empty()).unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    assert_security_headers(&res);
}

#[tokio::test]
async fn test_security_headers_present_on_401_unauthorized() {
    setup();
    let app = setup_app().await;

    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    assert_security_headers(&res);
}

#[tokio::test]
async fn test_security_headers_present_on_403_forbidden() {
    setup();
    let app = setup_app().await;

    // Reserved tenant_id "system" is rejected by TenantId validation in auth flow.
    let token = generate_token_with_claims(true, Some("active"), "system", Some("user_123"));
    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Authorization", format!("Bearer {token}"))
        .header("Idempotency-Key", "sec-test-403")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    assert_security_headers(&res);
}

#[tokio::test]
async fn test_security_headers_present_on_404_not_found() {
    setup();
    let app = setup_app().await;

    let mut builder = Request::builder().uri("/nonexistent-route").method("GET");
    #[cfg(debug_assertions)]
    {
        builder = builder.header("X-Tenant-Id", "tenant-1");
        builder = builder.header("X-User-Id", "user-1");
    }
    #[cfg(not(debug_assertions))]
    {
        let token = generate_token(true);
        builder = builder.header("Authorization", format!("Bearer {token}"));
    }
    let req = builder.body(Body::empty()).unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    assert_security_headers(&res);
}

fn assert_security_headers(res: &axum::response::Response) {
    let headers = res.headers();

    assert_eq!(headers.get("x-frame-options").unwrap(), "DENY");
    assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(
        headers.get("strict-transport-security").unwrap(),
        "max-age=31536000; includeSubDomains; preload"
    );
    assert_eq!(
        headers.get("content-security-policy").unwrap(),
        "default-src 'none'; frame-ancestors 'none';"
    );
    assert_eq!(
        headers.get("cross-origin-opener-policy").unwrap(),
        "same-origin"
    );
    assert_eq!(
        headers.get("cross-origin-embedder-policy").unwrap(),
        "require-corp"
    );
    assert_eq!(
        headers.get("cross-origin-resource-policy").unwrap(),
        "same-origin"
    );
}
