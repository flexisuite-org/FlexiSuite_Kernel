use super::connection::{RawConnection, TenantScoped};
use crate::entities::prelude::*;
use crate::entities::entity_record;
use crate::entities::entity_history;
use crate::entities::audit_log;
use async_trait::async_trait;
use kernel_core::kernel::{self, KernelError};
use sea_orm::{ActiveModelTrait, EntityTrait, ActiveValue};
use uuid::Uuid;

/// Sealed trait to prevent external implementations.
pub(crate) mod private {
    pub trait Sealed {}
}

/// The public interface for tenant-scoped database operations.
/// This trait is sealed and can only be implemented within this crate.
#[async_trait]
pub trait TenantRepository: private::Sealed + Send + Sync {
    async fn create_entity(&self, active_model: entity_record::ActiveModel) -> kernel::Result<entity_record::Model>;
    async fn update_entity(&self, active_model: entity_record::ActiveModel) -> kernel::Result<entity_record::Model>;
    async fn get_entity(&self, id: &str) -> kernel::Result<Option<entity_record::Model>>;
    async fn log_audit(&self, action: String, resource: String, details: serde_json::Value) -> kernel::Result<()>;
}

#[async_trait]
impl TenantRepository for TenantScoped<RawConnection> {
    async fn create_entity(&self, mut active_model: entity_record::ActiveModel) -> kernel::Result<entity_record::Model> {
        // Enforce tenant scoping by overriding the tenant_id field
        active_model.tenant_id = ActiveValue::Set(self.tenant_id.to_string());
        
        // Ensure ID is set
        let entity_id = match active_model.id.clone() {
             ActiveValue::Set(v) => v,
             ActiveValue::Unchanged(v) => v,
             ActiveValue::NotSet => return Err(KernelError::ValidationError("Entity ID is required".into())),
        };

        let entity_type = match active_model.entity_type.clone() {
             ActiveValue::Set(v) => v,
             ActiveValue::Unchanged(v) => v,
             ActiveValue::NotSet => return Err(KernelError::ValidationError("Entity Type is required".into())),
        };

        let content = match active_model.content.clone() {
             ActiveValue::Set(v) => v,
             ActiveValue::Unchanged(v) => v,
             ActiveValue::NotSet => return Err(KernelError::ValidationError("Content is required".into())),
        };

        let version = match active_model.version.clone() {
             ActiveValue::Set(v) => v,
             ActiveValue::Unchanged(v) => v,
             ActiveValue::NotSet => 1,
        };

        // User ID
        let user_id_str = self.user_id.as_ref().map(|u| u.to_string());

        // 1. Insert Entity
        let result = active_model.insert(&self.inner.txn).await.map_err(KernelError::db_error)?;

        // 2. Insert History
        let history = entity_history::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7().to_string()),
            tenant_id: ActiveValue::Set(self.tenant_id.to_string()),
            entity_id: ActiveValue::Set(entity_id),
            entity_type: ActiveValue::Set(entity_type),
            change_type: ActiveValue::Set("CREATE".to_string()),
            version: ActiveValue::Set(version),
            diff: ActiveValue::Set(content),
            created_at: ActiveValue::Set(chrono::Utc::now().into()),
            created_by: ActiveValue::Set(user_id_str),
            archived_at: ActiveValue::NotSet,
        };
        history.insert(&self.inner.txn).await.map_err(KernelError::db_error)?;

        Ok(result)
    }

    async fn update_entity(&self, mut active_model: entity_record::ActiveModel) -> kernel::Result<entity_record::Model> {
        // Enforce tenant scoping
        active_model.tenant_id = ActiveValue::Set(self.tenant_id.to_string());

        let entity_id = match active_model.id.clone() {
             ActiveValue::Set(v) => v,
             ActiveValue::Unchanged(v) => v,
             ActiveValue::NotSet => return Err(KernelError::ValidationError("Entity ID is required".into())),
        };

        // User ID
        let user_id_str = self.user_id.as_ref().map(|u| u.to_string());

        // 1. Update Entity
        let result = active_model.update(&self.inner.txn).await.map_err(KernelError::db_error)?;

        // 2. Insert History
        let history = entity_history::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7().to_string()),
            tenant_id: ActiveValue::Set(self.tenant_id.to_string()),
            entity_id: ActiveValue::Set(entity_id),
            entity_type: ActiveValue::Set(result.entity_type.clone()),
            change_type: ActiveValue::Set("UPDATE".to_string()),
            version: ActiveValue::Set(result.version),
            diff: ActiveValue::Set(result.content.clone()),
            created_at: ActiveValue::Set(chrono::Utc::now().into()),
            created_by: ActiveValue::Set(user_id_str),
            archived_at: ActiveValue::NotSet,
        };
        history.insert(&self.inner.txn).await.map_err(KernelError::db_error)?;

        Ok(result)
    }

    async fn get_entity(&self, id: &str) -> kernel::Result<Option<entity_record::Model>> {
        // Since we have a composite primary key (id, tenant_id), we must provide both.
        // RLS will also filter this, but SeaORM requires both for the PK lookup.
        let result = EntityRecord::find_by_id((id.to_string(), self.tenant_id.as_str().to_string()))
            .one(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)?;
        Ok(result)
    }

    async fn log_audit(&self, action: String, resource: String, details: serde_json::Value) -> kernel::Result<()> {
        let user_id_str = self.user_id.as_ref().map(|u| u.to_string()).unwrap_or_else(|| "unknown".to_string());

        let log = audit_log::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7().to_string()),
            tenant_id: ActiveValue::Set(self.tenant_id.to_string()),
            actor_id: ActiveValue::Set(user_id_str),
            action: ActiveValue::Set(action),
            resource: ActiveValue::Set(resource),
            details: ActiveValue::Set(details),
            ip_address: ActiveValue::NotSet,
            user_agent: ActiveValue::NotSet,
            created_at: ActiveValue::Set(chrono::Utc::now().into()),
            archived_at: ActiveValue::NotSet,
        };
        log.insert(&self.inner.txn).await.map_err(KernelError::db_error)?;
        Ok(())
    }
}
