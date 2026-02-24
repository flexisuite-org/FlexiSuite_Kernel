pub mod key_manager;

// Re-export KeyManager
pub use key_manager::{KeyManager, KeyManagerError};

// Re-export types from kernel-data
pub use kernel_data::auth_context::{
    SystemTenantContext, TenantContext, TenantId, UserId, is_valid_principal,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct RoleId(pub uuid::Uuid);

impl From<uuid::Uuid> for RoleId {
    fn from(uuid: uuid::Uuid) -> Self {
        Self(uuid)
    }
}

impl From<RoleId> for uuid::Uuid {
    fn from(id: RoleId) -> Self {
        id.0
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct PermissionId(pub uuid::Uuid);

impl From<uuid::Uuid> for PermissionId {
    fn from(uuid: uuid::Uuid) -> Self {
        Self(uuid)
    }
}

impl From<PermissionId> for uuid::Uuid {
    fn from(id: PermissionId) -> Self {
        id.0
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct GroupId(pub uuid::Uuid);

impl From<uuid::Uuid> for GroupId {
    fn from(uuid: uuid::Uuid) -> Self {
        Self(uuid)
    }
}

impl From<GroupId> for uuid::Uuid {
    fn from(id: GroupId) -> Self {
        id.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub id: RoleId,
    pub tenant_id: TenantId,
    pub name: String,
    pub description: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    pub id: PermissionId,
    pub tenant_id: TenantId,
    pub role_id: RoleId,
    pub resource: String,
    pub action: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub tenant_id: TenantId,
    pub name: String,
    pub description: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
