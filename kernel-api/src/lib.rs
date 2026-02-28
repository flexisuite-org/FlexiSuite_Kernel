use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::{HeaderName, HeaderValue, StatusCode},
    middleware::{from_fn, from_fn_with_state},
    routing::{get, post},
    response::IntoResponse,
};
use sea_orm::DatabaseConnection;
use serde::Serialize;
use std::sync::Arc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::auth::{TenantContext, auth_middleware};
use crate::middleware::{
    ActionStatus, MiddlewareConfig, MiddlewareState, get_action, idempotency_middleware,
    quota_middleware, record_action, load_permissions_middleware, require_permission,
};

pub mod auth;
pub mod diagnostics;
pub mod middleware;
pub mod profile;

// Re-export entities from kernel-data for use in tests and other consumers
#[cfg(feature = "test-utils")]
pub use kernel_data::entities;

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

    let public_router = Router::new().route("/health", get(readiness_check)).with_state(db.clone());

    // Reordered Middleware Stack:
    // 1. Auth (Outermost, establishes identity)
    // 2. Quota (Protect system resources)
    // 3. Idempotency (Handle replays early to avoid DB work)
    // 4. Permissions (RBAC, requires DB access via load_permissions_middleware)

    let require_perm = |p: &'static str| from_fn(move |req, next| require_permission(p, req, next));

    #[allow(unused_mut)]
    let mut protected_router = Router::new()
        .route("/test", post(write_test).put(write_test).layer(require_perm("test:write")))
        .route("/actions/:action_id", get(get_action_status).layer(require_perm("action:read")))
        // Diagnostics routes under /api/v1/diagnostics
        // Note: diagnostics routes implement their own policy checks, but we add a base permission check here as requested.
        .nest("/api/v1/diagnostics", diagnostics::routes().layer(require_perm("diagnostics:read")))
        // Outermost applied last
        .layer(from_fn(load_permissions_middleware))
        .layer(from_fn(idempotency_middleware))
        .layer(from_fn(quota_middleware))
        .layer(from_fn_with_state(db.clone(), auth_middleware));

    #[cfg(feature = "test-utils")]
    {
        protected_router = protected_router.route("/test/protected", get(|| async { "Access Granted" }).layer(require_perm("test:read")));
    }

    (
        Router::new()
            .merge(public_router)
            .merge(protected_router)
            .layer(Extension(state)),
        cleanup_handle,
    )
}

async fn readiness_check(State(db): State<Arc<DatabaseConnection>>) -> impl IntoResponse {
    // Check DB connectivity
    if let Err(e) = db.ping().await {
        tracing::error!("Readiness check failed (DB): {}", e);
        return (StatusCode::SERVICE_UNAVAILABLE, "Unhealthy (DB)").into_response();
    }

    // RBAC store is the DB, so if DB is up, RBAC storage is effectively up.
    // Real RBAC verification happens per-request with TenantContext.

    (StatusCode::OK, "OK").into_response()
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
