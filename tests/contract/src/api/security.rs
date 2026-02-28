use crate::api::middleware_integration::setup_app;
use crate::auth::helpers::setup;
// generate_token_with_claims is always needed for tests that need specific tenant/user claims.
// generate_token is only used when dev-auth feature is NOT enabled (real Bearer token path).
// When dev-auth is enabled, tests use X-Tenant-Id/X-User-Id debug headers instead.
#[cfg(not(feature = "dev-auth"))]
use crate::auth::helpers::{generate_token, generate_token_with_claims};
#[cfg(feature = "dev-auth")]
use crate::auth::helpers::generate_token_with_claims;
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
    #[cfg(feature = "dev-auth")]
    {
        builder = builder.header("X-Tenant-Id", "tenant-1");
        builder = builder.header("X-User-Id", "user-1");
    }
    #[cfg(not(feature = "dev-auth"))]
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

    // Test that reserved tenant_id "system" is rejected via real token validation.
    // We use generate_token_with_claims (PASETO) instead of dev headers (X-Tenant-Id)
    // because we need to exercise the actual TenantId validation logic in the auth flow,
    // not the debug bypass path that skips validation.
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
    #[cfg(feature = "dev-auth")]
    {
        builder = builder.header("X-Tenant-Id", "tenant-1");
        builder = builder.header("X-User-Id", "user-1");
    }
    #[cfg(not(feature = "dev-auth"))]
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

    assert_eq!(
        headers.get("x-frame-options").expect("missing header: x-frame-options"),
        "DENY"
    );
    assert_eq!(
        headers.get("x-content-type-options").expect("missing header: x-content-type-options"),
        "nosniff"
    );
    assert_eq!(
        headers.get("strict-transport-security").expect("missing header: strict-transport-security"),
        "max-age=31536000; includeSubDomains; preload"
    );
    assert_eq!(
        headers.get("content-security-policy").expect("missing header: content-security-policy"),
        "default-src 'none'; frame-ancestors 'none';"
    );
    assert_eq!(
        headers.get("cross-origin-opener-policy").expect("missing header: cross-origin-opener-policy"),
        "same-origin"
    );
    assert_eq!(
        headers.get("cross-origin-embedder-policy").expect("missing header: cross-origin-embedder-policy"),
        "require-corp"
    );
    assert_eq!(
        headers.get("cross-origin-resource-policy").expect("missing header: cross-origin-resource-policy"),
        "same-origin"
    );
}
