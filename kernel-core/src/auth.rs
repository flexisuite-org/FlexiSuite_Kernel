pub mod key_manager;

// Re-export KeyManager
pub use key_manager::{KeyManager, KeyManagerError};

// Re-export types from kernel-data
pub use kernel_data::auth_context::{
    SystemTenantContext, TenantContext, TenantId, UserId, is_valid_principal,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub id: uuid::Uuid,
    pub tenant_id: TenantId,
    pub name: String,
    pub description: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    pub id: uuid::Uuid,
    pub tenant_id: TenantId,
    pub role_id: uuid::Uuid,
    pub resource: String,
    pub action: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: uuid::Uuid,
    pub tenant_id: TenantId,
    pub name: String,
    pub description: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
