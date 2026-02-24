use axum::{
    Json, Router,
    extract::{Extension, Path},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    middleware::{from_fn, from_fn_with_state},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tower::ServiceBuilder;
use tower_http::set_header::SetResponseHeaderLayer;
use uuid::Uuid;

use crate::auth::{TenantContext, auth_middleware};
use crate::middleware::{
    ActionStatus, MiddlewareConfig, MiddlewareState, PingStatus, get_action,
    idempotency_middleware, quota_middleware, record_action,
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

    let public_router = Router::new()
        .route("/health", get(liveness))
        .route("/health/liveness", get(liveness));

    let protected_router = Router::new()
        .route("/test", post(write_test).put(write_test))
        .route("/actions/:action_id", get(get_action_status))
        .route("/health/readiness", get(readiness))
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
            .layer(Extension(db))
            .layer(
                ServiceBuilder::new()
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
    build_json_error_response("Action not found", StatusCode::NOT_FOUND, request_id)
}

#[derive(Serialize)]
struct JsonError {
    status: u16,
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
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

#[derive(Serialize)]
struct ReadinessResponse {
    status: String,
    checks: ReadinessChecks,
}

#[derive(Serialize)]
struct ReadinessChecks {
    database: Health,
    redis: Health,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Health {
    Up,
    Down,
    Degraded,
}

async fn liveness() -> StatusCode {
    StatusCode::OK
}

async fn readiness(
    Extension(state): Extension<MiddlewareState>,
    Extension(db): Extension<Arc<DatabaseConnection>>,
) -> Response {
    let db_timeout = Duration::from_secs(5);
    let redis_timeout = Duration::from_secs(5);

    let db_future = tokio::time::timeout(db_timeout, async move {
        let stmt = Statement::from_string(db.get_database_backend(), "SELECT 1".to_string());
        db.execute(stmt)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    });
    let redis_future = tokio::time::timeout(redis_timeout, state.idempotency_store.ping());

    let (db_res, redis_res) = tokio::join!(db_future, redis_future);

    let db_health = match db_res {
        Ok(Ok(_)) => Health::Up,
        Ok(Err(e)) => {
            tracing::error!(error = %e, "Readiness check failed (database)");
            Health::Down
        }
        Err(_) => {
            tracing::error!(
                "Readiness check timed out after {}s (database)",
                db_timeout.as_secs()
            );
            Health::Down
        }
    };

    let redis_health = match redis_res {
        Ok(Ok(PingStatus::Ok)) => Health::Up,
        Ok(Ok(PingStatus::Degraded)) => Health::Degraded,
        Ok(Err(e)) => {
            tracing::error!(error = ?e, "Readiness check failed (redis)");
            Health::Down
        }
        Err(_) => {
            tracing::error!(
                "Readiness check timed out after {}s (redis)",
                redis_timeout.as_secs()
            );
            Health::Down
        }
    };

    let status = if db_health == Health::Up && redis_health != Health::Down {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let body = ReadinessResponse {
        status: if status == StatusCode::OK {
            "healthy".to_string()
        } else {
            "unhealthy".to_string()
        },
        checks: ReadinessChecks {
            database: db_health,
            redis: redis_health,
        },
    };
    (status, Json(body)).into_response()
}
