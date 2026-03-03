use crate::api::middleware_integration::{setup_app, setup_app_with_db};
use crate::auth::helpers::setup;
// generate_token_with_claims is always needed for tests that need specific tenant/user claims.
// generate_token is only used when dev-auth feature is NOT enabled (real Bearer token path).
// When dev-auth is enabled, tests use X-Tenant-Id/X-User-Id debug headers instead.
#[cfg(not(feature = "dev-auth"))]
use crate::auth::helpers::{generate_token, generate_token_with_claims};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
#[cfg(not(feature = "dev-auth"))]
use chrono::Utc;
#[cfg(not(feature = "dev-auth"))]
use kernel_data::entities::{key_record, permission};
#[cfg(not(feature = "dev-auth"))]
use sea_orm::{MockDatabase, MockExecResult};
use tower::ServiceExt;
#[cfg(not(feature = "dev-auth"))]
use uuid::Uuid;

#[cfg(not(feature = "dev-auth"))]
async fn setup_app_for_bearer_tests() -> axum::Router {
    let now = Utc::now();

    let hmac_key = key_record::Model {
        kid: "hmac-key-1".to_string(),
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
        .append_query_results([[hmac_key]])
        .append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }])
        .append_query_results([[perm]])
        .into_connection();

    setup_app_with_db(db).await
}

#[tokio::test]
async fn test_security_headers_present_on_ok_201() {
    setup();
    #[cfg(not(feature = "dev-auth"))]
    let app = setup_app_for_bearer_tests().await;
    #[cfg(feature = "dev-auth")]
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
        .header("Idempotency-Key", "security-401-test")
        .body(Body::empty())
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    assert_security_headers(&res);
}

#[tokio::test]
#[cfg(not(feature = "dev-auth"))]
async fn test_security_headers_present_on_403_forbidden() {
    setup();
    let app = setup_app_for_bearer_tests().await;

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
    #[cfg(not(feature = "dev-auth"))]
    let app = setup_app_for_bearer_tests().await;
    #[cfg(feature = "dev-auth")]
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
