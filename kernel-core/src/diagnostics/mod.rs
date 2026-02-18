use crate::auth::{TenantId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub mod sanitizer;

/// Domain-layer diagnostic report representation.
///
/// Reserved for future domain-level logic (e.g., aggregation, analysis pipelines).
/// The persistence layer uses `kernel_data::entities::diagnostic_report::Model` for DB operations.
/// This struct coexists intentionally to maintain kernel/data separation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub trace_id: Uuid,
    pub tenant_id: TenantId,
    pub error_code: String,
    pub context: DiagnosticContext,
    pub suggestion: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Diagnostic context captured from the client.
///
/// # Size Constraints
/// - Request body size is enforced by API middleware (`max_body_size`, default 10MB).
/// - Individual string fields (e.g., `error_code`) are validated at the handler boundary.
/// - `dom_snapshot` and `props` are sanitized before storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticContext {
    pub component_id: String,
    pub props: Value,         // PII Masked
    pub dom_snapshot: String, // Sanitized & PII Scrubbed
    pub metrics: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticPolicy {
    pub tenant_id: TenantId,
    pub enabled: bool, // Default false (opt-out)
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<UserId>, // user_id or system
}

impl DiagnosticPolicy {
    pub fn new(tenant_id: TenantId, enabled: bool, updated_by: Option<UserId>) -> Self {
        Self {
            tenant_id,
            enabled,
            updated_at: Utc::now(),
            updated_by,
        }
    }
}
