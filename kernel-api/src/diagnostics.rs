use axum::{
    Router,
    extract::{Extension, Json, Query},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use kernel_core::auth::TenantContext;
use kernel_core::diagnostics::{DiagnosticContext, sanitizer::PIISanitizer};
use kernel_core::kernel::KernelError;
use kernel_data::{
    TenantRepository,
    entities::{diagnostic_policy, diagnostic_report},
    with_tenant_tx,
};
use sea_orm::{ActiveValue, DatabaseConnection};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

/// Maximum allowed length for user-supplied string fields (error_code, trace_id, etc.)
const MAX_STRING_LEN: usize = 256;
/// Maximum allowed length for DOM snapshot
const MAX_DOM_SNAPSHOT_LEN: usize = 1024 * 1024; // 1MB

fn validate_string_length(value: &str, field_name: &str) -> Result<(), StatusCode> {
    let max_len = if field_name == "dom_snapshot" {
        MAX_DOM_SNAPSHOT_LEN
    } else {
        MAX_STRING_LEN
    };

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

pub fn routes() -> Router {
    Router::new()
        .route("/report", post(report_diagnostic))
        .route("/query", get(query_diagnostic))
        .route("/health", get(get_health))
        .route("/policy", get(get_policy).put(update_policy))
}

async fn report_diagnostic(
    Extension(db): Extension<Arc<DatabaseConnection>>,
    Extension(ctx): Extension<TenantContext>,
    Json(mut payload): Json<ReportDiagnosticRequest>,
) -> impl IntoResponse {
    // 0. Validate input lengths
    if let Err(status) = validate_string_length(&payload.error_code, "error_code") {
        return status.into_response();
    }
    if let Err(status) = payload
        .suggestion
        .as_ref()
        .map(|s| validate_string_length(s, "suggestion"))
        .transpose()
        .map(|_| ())
    {
        return status.into_response();
    }
    if let Err(status) = validate_string_length(&payload.context.dom_snapshot, "dom_snapshot") {
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

    // 2. Check Policy & Save
    let result = with_tenant_tx(&db, &ctx, move |repo| {
        Box::pin(async move {
            let policy = repo.get_diagnostic_policy().await?;
            let enabled = policy.map(|p| p.enabled).unwrap_or(false);

            if !enabled {
                return Ok(None);
            }

            let context_value = serde_json::to_value(payload.context).map_err(|e| {
                KernelError::ValidationError(format!("failed to serialize diagnostic context: {e}"))
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
    })
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
    Extension(db): Extension<Arc<DatabaseConnection>>,
    Extension(ctx): Extension<TenantContext>,
    Query(payload): Query<QueryDiagnosticRequest>,
) -> impl IntoResponse {
    // Validate input length
    if let Err(status) = validate_string_length(&payload.trace_id, "trace_id") {
        return status.into_response();
    }

    let result = with_tenant_tx(&db, &ctx, move |repo| {
        Box::pin(async move { repo.get_diagnostic_report(&payload.trace_id).await })
    })
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

/// Stub health check endpoint. Does not perform real service checks.
async fn get_health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "score": 100,
        "stub": true
    }))
}

async fn get_policy(
    Extension(db): Extension<Arc<DatabaseConnection>>,
    Extension(ctx): Extension<TenantContext>,
) -> impl IntoResponse {
    let result = with_tenant_tx(&db, &ctx, move |repo| {
        Box::pin(async move { repo.get_diagnostic_policy().await })
    })
    .await;

    match result {
        Ok(Some(policy)) => Json(policy).into_response(),
        Ok(None) => Json(diagnostic_policy::Model {
            tenant_id: ctx.tenant_id().to_string(),
            enabled: false,
            updated_at: Utc::now().into(),
            updated_by: None,
        })
        .into_response(),
        Err(e) => {
            tracing::error!("Failed to get policy: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn update_policy(
    Extension(db): Extension<Arc<DatabaseConnection>>,
    Extension(ctx): Extension<TenantContext>,
    Json(payload): Json<UpdatePolicyRequest>,
) -> impl IntoResponse {
    let user_id = match ctx.user_id() {
        Some(uid) => uid.to_string(),
        None => return StatusCode::FORBIDDEN.into_response(),
    };

    let enabled = payload.enabled;
    let result = with_tenant_tx(&db, &ctx, move |repo| {
        Box::pin(async move {
            let model = diagnostic_policy::ActiveModel {
                tenant_id: ActiveValue::NotSet,
                enabled: ActiveValue::Set(enabled),
                updated_at: ActiveValue::Set(Utc::now().into()),
                updated_by: ActiveValue::Set(Some(user_id.clone())),
            };
            let policy = repo.upsert_diagnostic_policy(model).await?;

            // Record audit entry for policy change
            repo.log_audit(
                "update_policy".to_string(),
                "diagnostic_policy".to_string(),
                serde_json::json!({
                    "updated_by": user_id,
                    "field": "enabled",
                    "new_value": enabled,
                }),
            )
            .await?;

            Ok(policy)
        })
    })
    .await;

    match result {
        Ok(policy) => Json(policy).into_response(),
        Err(e) => {
            tracing::error!("Failed to update policy: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
