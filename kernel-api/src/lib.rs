use axum::{
    Json, Router,
    extract::{Extension, Path},
    http::{HeaderName, HeaderValue, StatusCode},
    middleware::{from_fn, from_fn_with_state},
    routing::{get, post},
};
use sea_orm::DatabaseConnection;
use serde::Serialize;
use std::sync::Arc;
use tokio::task::JoinHandle;
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

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
    pub action_id: Option<String>,
    pub tenant_id: String,
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

    // Public health endpoints (no auth required)
    let public_router = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/readiness", get(readiness));

    // Test endpoints - only available in test mode or with test-utils feature
    #[cfg(any(test, feature = "test-utils"))]
    let test_router = Router::new()
        .route("/test", post(write_test).put(write_test))
        .route("/actions/:action_id", get(get_action_status));

    #[cfg(not(any(test, feature = "test-utils")))]
    let test_router: Router<()> = Router::new();

    let protected_router = test_router
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
            .layer(Extension(state)),
        cleanup_handle,
    )
}

pub async fn readiness(Extension(_state): Extension<MiddlewareState>) -> StatusCode {
    StatusCode::OK
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
) -> Result<Json<ActionStatusResponse>, (StatusCode, Json<ErrorResponse>)> {
    if let Some(record) = get_action(&state, ctx.tenant_id().clone(), &action_id).await {
        return Ok(Json(ActionStatusResponse {
            action_id,
            status: record.status,
        }));
    }

    Err((
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "action not found".to_string(),
            code: "ACTION_NOT_FOUND".to_string(),
            action_id: Some(action_id),
            tenant_id: ctx.tenant_id().to_string(),
        }),
    ))
}
