use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    middleware::from_fn,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use uuid::Uuid;

use crate::auth::{auth_middleware, TenantContext};
use crate::middleware::{
    get_action, idempotency_middleware, quota_middleware, record_action, ActionStatus, MiddlewareState,
};

pub mod auth;
pub mod middleware;

#[derive(Serialize)]
struct TestWriteResponse {
    action_id: String,
    result_version: String,
    result: String,
}

#[derive(Serialize)]
struct ActionStatusResponse {
    action_id: String,
    status: ActionStatus,
}

pub fn build_app() -> Router {
    let state = MiddlewareState::new();
    state.start_cleanup_task();

    let public_router = Router::new().route("/health", get(|| async { "OK" }));

    let protected_router = Router::new()
        .route("/test", post(write_test).put(write_test))
        .route("/actions/:action_id", get(get_action_status))
        // Outermost applied last: Auth -> Idempotency -> Quota
        .layer(from_fn(quota_middleware))
        .layer(from_fn(idempotency_middleware))
        .layer(from_fn(auth_middleware));

    Router::new()
        .merge(public_router)
        .merge(protected_router)
        .layer(Extension(state))
}

async fn write_test(
    Extension(state): Extension<MiddlewareState>,
    Extension(ctx): Extension<TenantContext>,
) -> (StatusCode, [(String, String); 2], Json<TestWriteResponse>) {
    let action_id = Uuid::now_v7().to_string();
    record_action(&state.action_store, &ctx.tenant_id, &action_id, ActionStatus::Completed).await;

    let body = TestWriteResponse {
        action_id: action_id.clone(),
        result_version: "v1".to_string(),
        result: "OK".to_string(),
    };

    (
        StatusCode::CREATED,
        [
            ("X-Action-Id".to_string(), action_id),
            ("X-Result-Version".to_string(), "v1".to_string()),
        ],
        Json(body),
    )
}

async fn get_action_status(
    Path(action_id): Path<String>,
    Extension(state): Extension<MiddlewareState>,
    Extension(ctx): Extension<TenantContext>,
) -> Result<Json<ActionStatusResponse>, StatusCode> {
    if let Some(record) = get_action(&state.action_store, &ctx.tenant_id, &action_id).await {
        return Ok(Json(ActionStatusResponse {
            action_id,
            status: record.status,
        }));
    }

    Err(StatusCode::NOT_FOUND)
}
