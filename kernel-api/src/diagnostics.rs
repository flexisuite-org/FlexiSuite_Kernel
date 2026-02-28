#![allow(clippy::manual_inspect)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::manual_map)]
#![allow(clippy::collapsible_else_if)]
#![allow(clippy::implicit_saturating_sub)]
#![allow(clippy::needless_borrows_for_generic_args)]
use axum::{
    Router,
    extract::{Extension, Json, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use kernel_core::auth::{KeyManager, KeyManagerError, TenantContext};
use kernel_core::diagnostics::{DiagnosticContext, sanitizer::PIISanitizer};
use kernel_data::{
    DataError, TenantRepository, TenantScoped,
    connection::RawConnection,
    entities::{diagnostic_policy, diagnostic_report},
    with_tenant_tx,
};
use sea_orm::ActiveValue;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum allowed length for user-supplied string fields (error_code, trace_id, etc.)
const MAX_STRING_LEN: usize = 256;
/// Maximum allowed length for DOM snapshot
const MAX_DOM_SNAPSHOT_LEN: usize = 1024 * 1024; // 1MB

fn validate_string_length(value: &str, field_name: &str, max_len: usize) -> Result<(), StatusCode> {
    if value.len() > max_len {
        tracing::warn!(
            field = %field_name,
            len = value.len(),
            max = max_len,
            "Input exceeds maximum allowed length"
        );
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct ReportDiagnosticRequest {
    pub error_code: String,
    pub context: DiagnosticContext,
    pub suggestion: Option<String>,
}

#[derive(Deserialize)]
pub struct QueryDiagnosticRequest {
    pub trace_id: String,
}

#[derive(Deserialize)]
pub struct UpdatePolicyRequest {
    pub enabled: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
enum DiagnosticPolicyResponse {
    Configured(diagnostic_policy::Model),
    NotConfigured { tenant_id: String, enabled: bool },
}

pub fn routes() -> Router {
    Router::new()
        .route("/report", post(report_diagnostic))
        .route("/query", get(query_diagnostic))
        .route("/health", get(get_health))
        .route("/policy", get(get_policy).put(update_policy))
}

async fn generate_token_or_500(ctx: &TenantContext) -> Result<String, StatusCode> {
    match KeyManager::generate_tenant_token(ctx, ctx.tenant_id()).await {
        Ok(token) => Ok(token),
        Err(e) => {
            let status = match &e {
                KeyManagerError::NoActiveKey(_) => StatusCode::SERVICE_UNAVAILABLE,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            tracing::error!(error = %e, status = %status, "Failed to generate tenant token");
            Err(status)
        }
    }
}

async fn report_diagnostic(
    Extension(ctx): Extension<TenantContext>,
    Json(mut payload): Json<ReportDiagnosticRequest>,
) -> impl IntoResponse {
    // 0. Validate input lengths
    if let Err(status) = validate_string_length(&payload.error_code, "error_code", MAX_STRING_LEN) {
        return status.into_response();
    }
    if let Some(suggestion) = payload.suggestion.as_ref() {
        if let Err(status) = validate_string_length(suggestion, "suggestion", MAX_STRING_LEN) {
            return status.into_response();
        }
    }
    if let Err(status) = validate_string_length(
        &payload.context.dom_snapshot,
        "dom_snapshot",
        MAX_DOM_SNAPSHOT_LEN,
    ) {
        return status.into_response();
    }

    // 1. Sanitize (Defense in Depth)
    PIISanitizer::sanitize_value(&mut payload.context.props);
    payload.context.dom_snapshot = PIISanitizer::sanitize_text(&payload.context.dom_snapshot);
    if let Some(metrics) = &mut payload.context.metrics {
        PIISanitizer::sanitize_value(metrics);
    }
    payload.error_code = PIISanitizer::sanitize_text(&payload.error_code);
    payload.suggestion = if let Some(suggestion) = payload.suggestion.take() {
        Some(PIISanitizer::sanitize_text(&suggestion))
    } else {
        None
    };

    let trace_id = Uuid::now_v7();
    let token = match generate_token_or_500(&ctx).await {
        Ok(token) => token,
        Err(status) => return status.into_response(),
    };
    let db = match ctx.db() {
        Ok(db) => db,
        Err(e) => {
            tracing::error!(error = %e, "TenantContext missing database connection");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // 2. Check Policy & Save
    let result = with_tenant_tx(
        db,
        &ctx,
        &token,
        move |repo: &TenantScoped<RawConnection>| {
            Box::pin(async move {
                let policy = repo.get_diagnostic_policy().await?;
                let enabled = policy.map(|p| p.enabled).unwrap_or(false);

                if !enabled {
                    return Ok(None);
                }

                let context_value = serde_json::to_value(payload.context).map_err(|e| {
                    DataError::SerializationError(format!(
                        "failed to serialize diagnostic context: {e}"
                    ))
                })?;

                let report_model = diagnostic_report::ActiveModel {
                    trace_id: ActiveValue::Set(trace_id.to_string()),
                    tenant_id: ActiveValue::NotSet,
                    error_code: ActiveValue::Set(payload.error_code),
                    context: ActiveValue::Set(context_value),
                    suggestion: ActiveValue::Set(payload.suggestion),
                    created_at: ActiveValue::Set(Utc::now().into()),
                };

                let saved = repo.create_diagnostic_report(report_model).await?;
                Ok(Some(saved))
            })
        },
    )
    .await;

    match result {
        Ok(Some(saved)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"trace_id": saved.trace_id})),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::OK,
            Json(serde_json::json!({"status": "dropped_by_policy"})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("Failed to save diagnostic report: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn query_diagnostic(
    Extension(ctx): Extension<TenantContext>,
    Query(payload): Query<QueryDiagnosticRequest>,
) -> impl IntoResponse {
    // Validate input length
    if let Err(status) = validate_string_length(&payload.trace_id, "trace_id", MAX_STRING_LEN) {
        return status.into_response();
    }

    let token = match generate_token_or_500(&ctx).await {
        Ok(token) => token,
        Err(status) => return status.into_response(),
    };
    let db = match ctx.db() {
        Ok(db) => db,
        Err(e) => {
            tracing::error!(error = %e, "TenantContext missing database connection");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let result = with_tenant_tx(
        db,
        &ctx,
        &token,
        move |repo: &TenantScoped<RawConnection>| {
            Box::pin(async move { repo.get_diagnostic_report(&payload.trace_id).await })
        },
    )
    .await;

    match result {
        Ok(Some(report)) => Json(report).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("Failed to query diagnostic report: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Basic health check endpoint.
/// Checks connectivity to the primary database via TenantContext to enforce scoping.
async fn get_health(Extension(ctx): Extension<TenantContext>) -> impl IntoResponse {
    let db = match ctx.db() {
        Ok(db) => db,
        Err(e) => {
            tracing::error!("Health check failed (Context): {}", e);
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };

    match tokio::time::timeout(std::time::Duration::from_secs(1), db.ping()).await {
        Ok(Ok(_)) => (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response(),
        Ok(Err(e)) => {
            tracing::error!("Health check failed (DB): {}", e);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "error",
                    "details": "Database connectivity failed"
                })),
            )
                .into_response()
        }
        Err(_) => {
            tracing::error!("Health check timed out (DB)");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "status": "error",
                    "details": "Database connectivity failed"
                })),
            )
                .into_response()
        }
    }
}

async fn get_policy(Extension(ctx): Extension<TenantContext>) -> impl IntoResponse {
    let token = match generate_token_or_500(&ctx).await {
        Ok(token) => token,
        Err(status) => return status.into_response(),
    };
    let db = match ctx.db() {
        Ok(db) => db,
        Err(e) => {
            tracing::error!(error = %e, "TenantContext missing database connection");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let result = with_tenant_tx(
        db,
        &ctx,
        &token,
        move |repo: &TenantScoped<RawConnection>| {
            Box::pin(async move { repo.get_diagnostic_policy().await })
        },
    )
    .await;

    match result {
        Ok(Some(policy)) => Json(DiagnosticPolicyResponse::Configured(policy)).into_response(),
        Ok(None) => Json(DiagnosticPolicyResponse::NotConfigured {
            tenant_id: ctx.tenant_id().to_string(),
            enabled: false,
        })
        .into_response(),
        Err(e) => {
            tracing::error!("Failed to get policy: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn update_policy(
    Extension(_ctx): Extension<TenantContext>,
    Json(_payload): Json<UpdatePolicyRequest>,
) -> impl IntoResponse {
    // TODO(issue-rbac-update-policy-20260218): tenant-admin RBAC check for update_policy を実装する。
    // Ticket: .kiro/specs/coderabbit_jules_linkage/tickets/issue-rbac-update-policy-20260218.md
    StatusCode::FORBIDDEN.into_response()
}
