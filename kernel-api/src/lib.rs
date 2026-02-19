use axum::{
    Json, Router,
    extract::{Extension, Path},
    http::{HeaderName, HeaderValue, StatusCode, header},
    middleware::{from_fn, from_fn_with_state},
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
pub mod middleware;
pub mod profile;

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

    let public_router = Router::new().route("/health", get(|| async { "OK" }));

    let protected_router = Router::new()
        .route("/test", post(write_test).put(write_test))
        .route("/actions/:action_id", get(get_action_status))
        // Diagnostics routes under /api/v1/diagnostics
        .nest("/api/v1/diagnostics", diagnostics::routes())
        // Outermost applied last: Auth -> Idempotency -> Quota
        .layer(from_fn(quota_middleware))
        .layer(from_fn(idempotency_middleware))
        .layer(from_fn_with_state(db.clone(), auth_middleware));

    (
        Router::new()
            .merge(public_router)
            .merge(protected_router)
            .layer(Extension(state))
            .layer(
                ServiceBuilder::new()
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
                        HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
                    )),
            ),
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
    Extension(state): Extension<MiddlewareState>,
    Extension(ctx): Extension<TenantContext>,
) -> Result<Json<ActionStatusResponse>, StatusCode> {
    if let Some(record) = get_action(&state, ctx.tenant_id().clone(), &action_id).await {
        return Ok(Json(ActionStatusResponse {
            action_id,
            status: record.status,
        }));
    }

    Err(StatusCode::NOT_FOUND)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use sea_orm::{DatabaseBackend, MockDatabase};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_security_headers() {
        let db = Arc::new(MockDatabase::new(DatabaseBackend::Postgres).into_connection());
        let mut config = MiddlewareConfig::default();
        config.require_redis = false;
        // Use an invalid URL to force fallback to in-memory store without delay
        config.redis_url = "redis://0.0.0.0:0".to_string();

        let state = MiddlewareState::new(config)
            .await
            .expect("Failed to create state");

        let (app, _cleanup) = build_app_with_state(state, db);

        let request = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let headers = response.headers();
        assert_eq!(
            headers.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(),
            "nosniff"
        );
        assert_eq!(headers.get(header::X_FRAME_OPTIONS).unwrap(), "DENY");
        assert_eq!(
            headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
            "default-src 'none'; frame-ancestors 'none'"
        );
        assert_eq!(
            headers.get(header::STRICT_TRANSPORT_SECURITY).unwrap(),
            "max-age=63072000; includeSubDomains; preload"
        );
    }
}
