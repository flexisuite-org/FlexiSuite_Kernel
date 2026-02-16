use axum::{
    Json, Router,
    extract::{Extension, Path},
    http::{HeaderName, HeaderValue, StatusCode},
    middleware::from_fn,
    routing::{get, post},
};
use serde::Serialize;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::auth::{TenantContext, auth_middleware};
use crate::middleware::{
    ActionStatus, MiddlewareConfig, MiddlewareState, get_action, idempotency_middleware,
    quota_middleware, record_action,
};

pub mod auth;
pub mod middleware;

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

pub fn build_app(config: MiddlewareConfig) -> (Router, JoinHandle<()>) {
    build_app_with_state(MiddlewareState::new(config))
}

pub fn build_app_with_state(state: MiddlewareState) -> (Router, JoinHandle<()>) {
    let cleanup_handle = state.start_cleanup_task();

    let public_router = Router::new().route("/health", get(|| async { "OK" }));

    let protected_router = Router::new()
        .route("/test", post(write_test).put(write_test))
        .route("/actions/:action_id", get(get_action_status))
        // Outermost applied last: Auth -> Idempotency -> Quota
        .layer(from_fn(quota_middleware))
        .layer(from_fn(idempotency_middleware))
        .layer(from_fn(auth_middleware));

    (
        Router::new()
            .merge(public_router)
            .merge(protected_router)
            .layer(Extension(state)),
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
