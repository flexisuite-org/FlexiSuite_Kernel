use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use kernel_api::{build_app_with_state, middleware::MiddlewareConfig, middleware::MiddlewareState};
use sea_orm::{DatabaseBackend, MockDatabase};
use std::sync::Arc;
use tower::ServiceExt;
use http_body_util::BodyExt; // Need this to collect body

#[tokio::test]
async fn test_get_action_status_not_found_ux() {
    let db = Arc::new(MockDatabase::new(DatabaseBackend::Postgres).into_connection());
    let mut config = MiddlewareConfig::default();
    config.require_redis = false;
    config.redis_url = "redis://0.0.0.0:0".to_string();

    let state = MiddlewareState::new(config).await.expect("Failed to create state");
    let (app, _cleanup) = build_app_with_state(state, db);

    let request = Request::builder()
        .uri("/actions/non-existent-id")
        .header("X-Tenant-Id", "tenant-1")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.starts_with("application/json"),
        "Expected application/json Content-Type, got: {content_type}"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();

    // Expect JSON error response
    let json: serde_json::Value = serde_json::from_str(&body_str).expect("Body should be JSON");
    assert_eq!(json["error"], "Action not found");
}
