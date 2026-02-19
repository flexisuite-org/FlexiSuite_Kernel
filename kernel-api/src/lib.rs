use axum::{
    Json, Router,
    extract::{Extension, Path},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    middleware::{from_fn, from_fn_with_state},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use sea_orm::DatabaseConnection;
use serde::Serialize;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tower::ServiceBuilder;
use tower_http::set_header::SetResponseHeaderLayer;
use uuid::Uuid;

use crate::auth::{TenantContext, auth_middleware};
use crate::middleware::{
    ActionStatus, MiddlewareConfig, MiddlewareState, get_action, idempotency_middleware,
    quota_middleware, record_action,
};

pub mod auth;
pub mod diagnostics;
pub mod health;
pub mod middleware;
pub mod profile;
pub mod error;

#[derive(Serialize)]
pub struct TestWriteResponse {
    pub action_id: String,
    pub result_version: String,
    pub result: String,
}

#[derive(Serialize)]
pub struct ActionStatusResponse {
    pub action_id: String,
    pub status: ActionStatus,
}

pub async fn build_app(
    config: MiddlewareConfig,
    db: Arc<DatabaseConnection>,
) -> Result<(Router, JoinHandle<()>), String> {
    let state = MiddlewareState::new(config).await?;
    Ok(build_app_with_state(state, db))
}

pub fn build_app_with_state(
    state: MiddlewareState,
    db: Arc<DatabaseConnection>,
) -> (Router, JoinHandle<()>) {
    let cleanup_handle = state.start_cleanup_task();

    let public_router = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/health/liveness", get(health::liveness))
        .route("/health/readiness", get(health::readiness));

    let protected_router = Router::new()
        .route("/test", post(write_test).put(write_test))
        .route("/actions/:action_id", get(get_action_status))
        // Diagnostics routes under /api/v1/diagnostics
        .nest("/api/v1/diagnostics", diagnostics::routes())
        // Outermost applied last: Auth -> Idempotency -> Quota
        .route_layer(from_fn(quota_middleware))
        .route_layer(from_fn(idempotency_middleware))
        .route_layer(from_fn_with_state(db.clone(), auth_middleware));

    (
        Router::new()
            .merge(public_router)
            .merge(protected_router)
            .layer(Extension(state))
            .layer(
                ServiceBuilder::new()
                    // COOP/COEP/CORP headers for cross-origin isolation
                    .layer(SetResponseHeaderLayer::overriding(
                        HeaderName::from_static("cross-origin-opener-policy"),
                        HeaderValue::from_static("same-origin"),
                    ))
                    .layer(SetResponseHeaderLayer::overriding(
                        HeaderName::from_static("cross-origin-embedder-policy"),
                        HeaderValue::from_static("require-corp"),
                    ))
                    .layer(SetResponseHeaderLayer::overriding(
                        HeaderName::from_static("cross-origin-resource-policy"),
                        HeaderValue::from_static("same-origin"),
                    ))
                    // Standard security headers
                    .layer(SetResponseHeaderLayer::overriding(
                        header::X_CONTENT_TYPE_OPTIONS,
                        HeaderValue::from_static("nosniff"),
                    ))
                    .layer(SetResponseHeaderLayer::overriding(
                        header::X_FRAME_OPTIONS,
                        HeaderValue::from_static("DENY"),
                    ))
                    .layer(SetResponseHeaderLayer::overriding(
                        header::CONTENT_SECURITY_POLICY,
                        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
                    ))
                    .layer(SetResponseHeaderLayer::overriding(
                        header::STRICT_TRANSPORT_SECURITY,
                        HeaderValue::from_static("max-age=63072000; includeSubDomains"),
                    )),
            )
            .layer(Extension(db)),
        cleanup_handle,
    )
}

pub async fn write_test(
    Extension(state): Extension<MiddlewareState>,
    Extension(ctx): Extension<TenantContext>,
) -> (
    StatusCode,
    [(HeaderName, HeaderValue); 2],
    Json<TestWriteResponse>,
) {
    let action_id = Uuid::now_v7().to_string();
    record_action(
        &state,
        ctx.tenant_id().clone(),
        &action_id,
        ActionStatus::Completed,
    )
    .await;

    let body = TestWriteResponse {
        action_id: action_id.clone(),
        result_version: "v1".to_string(),
        result: "OK".to_string(),
    };

    (
        StatusCode::CREATED,
        [
            (
                HeaderName::from_static("x-action-id"),
                HeaderValue::from_str(&action_id).expect("UUID v7 is valid ASCII"),
            ),
            (
                HeaderName::from_static("x-result-version"),
                HeaderValue::from_static("v1"),
            ),
        ],
        Json(body),
    )
}

pub async fn get_action_status(
    Path(action_id): Path<String>,
    headers: HeaderMap,
    Extension(state): Extension<MiddlewareState>,
    Extension(ctx): Extension<TenantContext>,
) -> Response {
    if let Some(record) = get_action(&state, ctx.tenant_id().clone(), &action_id).await {
        return Json(ActionStatusResponse {
            action_id,
            status: record.status,
        })
        .into_response();
    }

    let request_id = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    error::build_json_error_response("Action not found", StatusCode::NOT_FOUND, request_id)
}

#[cfg(test)]
mod security_header_tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use sea_orm::{DatabaseBackend, MockDatabase};
    use tower::ServiceExt;

    async fn make_test_app() -> Router {
        let db = Arc::new(MockDatabase::new(DatabaseBackend::Postgres).into_connection());
        let mut config = MiddlewareConfig::default();
        config.require_redis = false;
        config.redis_url = "redis://0.0.0.0:0".to_string();

        let state = MiddlewareState::new(config)
            .await
            .expect("Failed to create state");
        let (app, _cleanup) = build_app_with_state(state, db);
        app
    }

    async fn assert_security_headers(response: Response) {
        let headers = response.headers();
        assert_eq!(
            headers
                .get("cross-origin-opener-policy")
                .and_then(|v| v.to_str().ok()),
            Some("same-origin")
        );
        assert_eq!(
            headers
                .get("cross-origin-embedder-policy")
                .and_then(|v| v.to_str().ok()),
            Some("require-corp")
        );
        assert_eq!(
            headers
                .get("cross-origin-resource-policy")
                .and_then(|v| v.to_str().ok()),
            Some("same-origin")
        );

        // Verify existing security headers are present
        assert!(headers
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok())
            .is_some());
        assert!(headers
            .get("x-frame-options")
            .and_then(|v| v.to_str().ok())
            .is_some());
        assert!(headers
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok())
            .is_some());
        assert!(headers
            .get("strict-transport-security")
            .and_then(|v| v.to_str().ok())
            .is_some());
    }

    #[tokio::test]
    async fn test_security_headers_health() {
        let app = make_test_app().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_security_headers(response).await;
    }

    #[tokio::test]
    async fn test_security_headers_protected() {
        let app = make_test_app().await;
        // Call protected route without auth to trigger a response (even if it's 401/error, headers should still be there)
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .method("POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should be 401 Unauthorized because we didn't provide any auth headers
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_security_headers(response).await;
    }

    #[tokio::test]
    async fn test_security_headers_error() {
        let app = make_test_app().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/non-existent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_security_headers(response).await;
    }
}
