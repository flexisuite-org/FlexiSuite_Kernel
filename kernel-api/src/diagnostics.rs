use axum::{
    extract::{Json, Extension},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use kernel_core::auth::TenantContext;
use kernel_core::diagnostics::{DiagnosticContext, sanitizer::PIISanitizer};
use kernel_data::{with_tenant_tx, TenantRepository, entities::{diagnostic_report, diagnostic_policy}};
use sea_orm::{ActiveValue, DatabaseConnection};
use uuid::Uuid;
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

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
        .route("/query", post(query_diagnostic))
        .route("/health", get(get_health))
        .route("/policy", get(get_policy).put(update_policy))
}

async fn report_diagnostic(
    Extension(db): Extension<Arc<DatabaseConnection>>,
    Extension(ctx): Extension<TenantContext>,
    Json(mut payload): Json<ReportDiagnosticRequest>,
) -> impl IntoResponse {
    // 1. Sanitize (Defense in Depth)
    PIISanitizer::sanitize_value(&mut payload.context.props);
    payload.context.dom_snapshot = PIISanitizer::sanitize_text(&payload.context.dom_snapshot);
    if let Some(metrics) = &mut payload.context.metrics {
        PIISanitizer::sanitize_value(metrics);
    }

    let trace_id = Uuid::now_v7();

    // 2. Check Policy & Save
    let result = with_tenant_tx(&db, &ctx, move |repo| Box::pin(async move {
        let policy = repo.get_diagnostic_policy().await?;
        let enabled = policy.map(|p| p.enabled).unwrap_or(false);

        if !enabled {
             return Ok(None);
        }

        let report_model = diagnostic_report::ActiveModel {
            trace_id: ActiveValue::Set(trace_id.to_string()),
            tenant_id: ActiveValue::NotSet,
            error_code: ActiveValue::Set(payload.error_code),
            context: ActiveValue::Set(serde_json::to_value(payload.context).unwrap()),
            suggestion: ActiveValue::Set(payload.suggestion),
            created_at: ActiveValue::Set(Utc::now().into()),
        };

        let saved = repo.create_diagnostic_report(report_model).await?;
        Ok(Some(saved))
    })).await;

    match result {
        Ok(Some(saved)) => (StatusCode::CREATED, Json(serde_json::json!({"trace_id": saved.trace_id}))).into_response(),
        Ok(None) => (StatusCode::OK, Json(serde_json::json!({"status": "dropped_by_policy"}))).into_response(),
        Err(e) => {
            tracing::error!("Failed to save diagnostic report: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn query_diagnostic(
    Extension(db): Extension<Arc<DatabaseConnection>>,
    Extension(ctx): Extension<TenantContext>,
    Json(payload): Json<QueryDiagnosticRequest>,
) -> impl IntoResponse {
    let result = with_tenant_tx(&db, &ctx, move |repo| Box::pin(async move {
        repo.get_diagnostic_report(&payload.trace_id).await
    })).await;

    match result {
        Ok(Some(report)) => Json(report).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!("Failed to query diagnostic report: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn get_health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "score": 100
    }))
}

async fn get_policy(
    Extension(db): Extension<Arc<DatabaseConnection>>,
    Extension(ctx): Extension<TenantContext>,
) -> impl IntoResponse {
    let result = with_tenant_tx(&db, &ctx, move |repo| Box::pin(async move {
        repo.get_diagnostic_policy().await
    })).await;

    match result {
        Ok(Some(policy)) => Json(policy).into_response(),
        Ok(None) => {
            Json(serde_json::json!({
                "tenant_id": ctx.tenant_id().as_str(),
                "enabled": false
            })).into_response()
        },
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

    let result = with_tenant_tx(&db, &ctx, move |repo| Box::pin(async move {
        let model = diagnostic_policy::ActiveModel {
            tenant_id: ActiveValue::NotSet,
            enabled: ActiveValue::Set(payload.enabled),
            updated_at: ActiveValue::Set(Utc::now().into()),
            updated_by: ActiveValue::Set(Some(user_id)),
        };
        repo.upsert_diagnostic_policy(model).await
    })).await;

    match result {
        Ok(policy) => Json(policy).into_response(),
        Err(e) => {
            tracing::error!("Failed to update policy: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
