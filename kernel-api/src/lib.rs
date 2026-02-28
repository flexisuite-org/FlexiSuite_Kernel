use axum::{
    Json, Router,
    extract::{Extension, Path},
    http::{HeaderName, HeaderValue, StatusCode},
    middleware::{from_fn, from_fn_with_state},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use sea_orm::DatabaseConnection;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tower_http::set_header::SetResponseHeaderLayer;
use uuid::Uuid;

use crate::auth::{TenantContext, auth_middleware};
use crate::middleware::{
    ActionStatus, MiddlewareConfig, MiddlewareState, get_action, idempotency_middleware,
    load_permissions_middleware, quota_middleware, record_action, require_permission,
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

#[derive(Serialize)]
struct JsonError {
    status: u16,
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
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
        .route("/health", get(liveness))
        .route("/health/liveness", get(liveness));

    let auth_only_router = Router::new()
        .route("/health/readiness", get(readiness))
        .layer(from_fn_with_state(db.clone(), auth_middleware));

    // Middleware chain (outermost -> innermost):
    // Auth -> Idempotency -> Quota -> Permissions

    let require_perm = |p: &'static str| from_fn(move |req, next| require_permission(p, req, next));

    #[allow(unused_mut)]
    let mut protected_router = Router::new()
        .route(
            "/test",
            post(write_test)
                .put(write_test)
                .layer(require_perm("test:write")),
        )
        .route(
            "/actions/:action_id",
            get(get_action_status).layer(require_perm("action:read")),
        )
        // Diagnostics routes under /api/v1/diagnostics
        // Note: diagnostics routes implement their own policy checks, but we add a base permission check here as requested.
        .nest(
            "/api/v1/diagnostics",
            diagnostics::routes().layer(require_perm("diagnostics:read")),
        )
        // Outermost applied last
        .layer(from_fn(load_permissions_middleware))
        .layer(from_fn(quota_middleware))
        .layer(from_fn(idempotency_middleware))
        .layer(from_fn_with_state(db.clone(), auth_middleware));

    #[cfg(feature = "test-utils")]
    {
        protected_router = protected_router.route(
            "/test/protected",
            get(|| async { "Access Granted" }).layer(require_perm("test:read")),
        );
    }

    (
        Router::new()
            .merge(public_router)
            .merge(auth_only_router)
            .merge(protected_router)
            .layer(Extension(state))
            .layer(Extension(db))
            .layer(SetResponseHeaderLayer::overriding(
                HeaderName::from_static("x-frame-options"),
                HeaderValue::from_static("DENY"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                HeaderName::from_static("x-content-type-options"),
                HeaderValue::from_static("nosniff"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                HeaderName::from_static("strict-transport-security"),
                HeaderValue::from_static("max-age=31536000; includeSubDomains; preload"),
            ))
            .layer(SetResponseHeaderLayer::overriding(
                HeaderName::from_static("content-security-policy"),
                HeaderValue::from_static("default-src 'none'; frame-ancestors 'none';"),
            ))
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
            )),
        cleanup_handle,
    )
}

async fn liveness() -> StatusCode {
    StatusCode::OK
}

fn readiness_db_timeout() -> Duration {
    std::env::var("HEALTH_READINESS_DB_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(1))
}

async fn readiness(Extension(ctx): Extension<TenantContext>) -> impl IntoResponse {
    let timeout = readiness_db_timeout();
    let result = ctx
        .with_system_context(|system_ctx| async move {
            let db = system_ctx.db().map_err(|e| format!("context error: {e}"))?;
            match tokio::time::timeout(timeout, db.ping()).await {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(e)) => Err(format!("db ping failed: {e}")),
                Err(_) => Err(format!(
                    "db ping timed out after {}ms",
                    timeout.as_millis()
                )),
            }
        })
        .await;

    match result {
        Ok(Ok(())) => (StatusCode::OK, "OK").into_response(),
        Ok(Err(e)) => {
            tracing::error!("Readiness check failed (system context): {e}");
            (StatusCode::SERVICE_UNAVAILABLE, "Unhealthy (DB)").into_response()
        }
        Err(e) => {
            tracing::error!("Readiness check failed (context): {e}");
            (StatusCode::SERVICE_UNAVAILABLE, "Unhealthy (Context)").into_response()
        }
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

pub fn build_json_error_response(
    message: impl Into<String>,
    status: StatusCode,
    request_id: Option<String>,
) -> Response {
    let body = JsonError {
        status: status.as_u16(),
        error: message.into(),
        request_id,
    };
    (status, Json(body)).into_response()
}
