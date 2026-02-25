use axum::{
    Json, Router,
    extract::{Extension, Path},
    http::{HeaderName, HeaderValue, StatusCode},
    middleware::{from_fn, from_fn_with_state},
    response::IntoResponse,
    routing::{get, post},
};
use sea_orm::DatabaseConnection;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use uuid::Uuid;

use crate::auth::{TenantContext, auth_middleware};
use crate::middleware::{
    ActionStatus, MiddlewareConfig, MiddlewareState, get_action, idempotency_middleware,
    load_permissions_middleware, quota_middleware, record_action,
};

#[cfg(feature = "test-utils")]
use crate::middleware::require_permission;

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

    let public_router = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/readiness", get(readiness_handler));

    // Reordered Middleware Stack:
    // 1. Auth (Outermost, establishes identity)
    // 2. Quota (Protect system resources)
    // 3. Idempotency (Handle replays early to avoid DB work)
    // 4. Permissions (RBAC, requires DB access via load_permissions_middleware)

    let mut protected_router = Router::new()
        .route("/test", post(write_test).put(write_test))
        .route("/actions/:action_id", get(get_action_status))
        // Diagnostics routes under /api/v1/diagnostics
        .nest("/api/v1/diagnostics", diagnostics::routes())
        // Outermost applied last
        .layer(from_fn(load_permissions_middleware))
        .layer(from_fn(idempotency_middleware))
        .layer(from_fn(quota_middleware))
        .layer(from_fn_with_state(db.clone(), auth_middleware));

    #[cfg(feature = "test-utils")]
    {
        protected_router = protected_router.route(
            "/test/protected",
            get(|| async { "Access Granted" }).layer(from_fn(|req, next| {
                require_permission("test:read", req, next)
            })),
        );
    }

    (
        Router::new()
            .merge(public_router)
            .merge(protected_router)
            .layer(Extension(db))
            .layer(Extension(state)),
        cleanup_handle,
    )
}

#[derive(Serialize)]
struct ReadinessResponse {
    status: &'static str,
    checks: Vec<(&'static str, &'static str)>,
}

async fn readiness_handler(Extension(db): Extension<Arc<DatabaseConnection>>) -> impl IntoResponse {
    const DB_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
    const AUTH_PROBE_TIMEOUT: Duration = Duration::from_secs(1);

    let mut checks: Vec<(&'static str, &'static str)> = Vec::new();
    let mut all_ok = true;

    match timeout(DB_PROBE_TIMEOUT, db.ping()).await {
        Ok(Ok(())) => checks.push(("database", "ok")),
        Ok(Err(e)) => {
            tracing::error!(error = %e, "readiness database probe failed");
            checks.push(("database", "failed"));
            all_ok = false;
        }
        Err(_) => {
            tracing::error!("readiness database probe timed out");
            checks.push(("database", "timeout"));
            all_ok = false;
        }
    }

    match timeout(AUTH_PROBE_TIMEOUT, async {
        crate::auth::is_auth_config_ready()
    })
    .await
    {
        Ok(true) => checks.push(("auth_config", "ok")),
        Ok(false) => {
            tracing::error!("readiness auth config probe failed: auth config not initialized");
            checks.push(("auth_config", "failed"));
            all_ok = false;
        }
        Err(_) => {
            tracing::error!("readiness auth config probe timed out");
            checks.push(("auth_config", "timeout"));
            all_ok = false;
        }
    }

    if all_ok {
        (
            StatusCode::OK,
            Json(ReadinessResponse {
                status: "ready",
                checks,
            }),
        )
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ReadinessResponse {
                status: "not_ready",
                checks,
            }),
        )
    }
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
