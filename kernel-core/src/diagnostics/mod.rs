use serde::{Deserialize, Serialize};
use crate::auth::{TenantId, UserId};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use serde_json::Value;

pub mod sanitizer;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub trace_id: Uuid,
    pub tenant_id: TenantId,
    pub error_code: String,
    pub context: DiagnosticContext,
    pub suggestion: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticContext {
    pub component_id: String,
    pub props: Value, // PII Masked
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
