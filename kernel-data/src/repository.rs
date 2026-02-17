use super::connection::{RawConnection, TenantScoped};
use crate::entities::audit_log;
use crate::entities::entity_history;
use crate::entities::entity_record;
use crate::entities::prelude::*;
use async_trait::async_trait;
use kernel_core::kernel::{self, KernelError};
use sea_orm::{ActiveModelTrait, ActiveValue, EntityTrait};
use uuid::Uuid;

const MAX_LOG_CHARS: usize = 256;

/// Sealed trait to prevent external implementations.
pub(crate) mod private {
    pub trait Sealed {}
}

/// The public interface for tenant-scoped database operations.
/// This trait is sealed and can only be implemented within this crate.
#[async_trait]
pub trait TenantRepository: private::Sealed + Send + Sync {
    async fn create_entity(
        &self,
        active_model: entity_record::ActiveModel,
    ) -> kernel::Result<entity_record::Model>;
    async fn update_entity(
        &self,
        active_model: entity_record::ActiveModel,
    ) -> kernel::Result<entity_record::Model>;
    async fn get_entity(&self, id: &str) -> kernel::Result<Option<entity_record::Model>>;
    async fn log_audit(
        &self,
        action: String,
        resource: String,
        details: serde_json::Value,
    ) -> kernel::Result<()>;
}

// Helper to sanitize log entries to satisfy CodeQL.
// In reality, DB writes are safe from log injection, but this ensures no control chars.
fn sanitize_for_log(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(MAX_LOG_CHARS)
        .collect()
}

fn required_active_value<T: Clone>(
    value: &ActiveValue<T>,
    field_name: &str,
) -> kernel::Result<T>
where
    T: Into<sea_orm::Value>,
{
    match value {
        ActiveValue::Set(v) | ActiveValue::Unchanged(v) => Ok(v.clone()),
        ActiveValue::NotSet => Err(KernelError::ValidationError(format!(
            "{field_name} is required"
        ))),
    }
}

fn active_value_or<T: Clone>(value: &ActiveValue<T>, default: T) -> T
where
    T: Into<sea_orm::Value>,
{
    match value {
        ActiveValue::Set(v) | ActiveValue::Unchanged(v) => v.clone(),
        ActiveValue::NotSet => default,
    }
}

#[async_trait]
impl TenantRepository for TenantScoped<RawConnection> {
    async fn create_entity(
        &self,
        mut active_model: entity_record::ActiveModel,
    ) -> kernel::Result<entity_record::Model> {
        // Enforce tenant scoping by overriding the tenant_id field
        active_model.tenant_id = ActiveValue::Set(self.tenant_id.to_string());

        // Ensure ID is set
        let entity_id = required_active_value(&active_model.id, "Entity ID")?;
        let entity_type = required_active_value(&active_model.entity_type, "Entity Type")?;
        let content = required_active_value(&active_model.content, "Content")?;
        let version = active_value_or(&active_model.version, 1);

        // User ID
        let user_id_str = self.user_id.as_ref().map(|u| sanitize_for_log(u.as_str()));

        // 1. Insert Entity
        let result = active_model
            .insert(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)?;

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
        history
            .insert(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)?;

        Ok(result)
    }

    async fn update_entity(
        &self,
        mut active_model: entity_record::ActiveModel,
    ) -> kernel::Result<entity_record::Model> {
        // Enforce tenant scoping
        active_model.tenant_id = ActiveValue::Unchanged(self.tenant_id.to_string());

        let entity_id = required_active_value(&active_model.id, "Entity ID")?;

        let existing = EntityRecord::find_by_id((entity_id.clone(), self.tenant_id.to_string()))
            .one(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)?
            .ok_or_else(|| KernelError::ValidationError("Entity not found".into()))?;

        let next_version = match active_model.version.clone() {
            ActiveValue::Set(v) | ActiveValue::Unchanged(v) => v + 1,
            ActiveValue::NotSet => existing.version + 1,
        };
        active_model.version = ActiveValue::Set(next_version);
        active_model.updated_at = ActiveValue::Set(chrono::Utc::now().into());

        // User ID
        let user_id_str = self.user_id.as_ref().map(|u| sanitize_for_log(u.as_str()));

        // 1. Update Entity
        let result = active_model
            .update(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)?;

        // 2. Insert History
        let history = entity_history::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7().to_string()),
            tenant_id: ActiveValue::Set(self.tenant_id.to_string()),
            entity_id: ActiveValue::Set(entity_id),
            entity_type: ActiveValue::Set(result.entity_type.clone()),
            change_type: ActiveValue::Set("UPDATE".to_string()),
            version: ActiveValue::Set(next_version),
            diff: ActiveValue::Set(result.content.clone()),
            created_at: ActiveValue::Set(chrono::Utc::now().into()),
            created_by: ActiveValue::Set(user_id_str),
            archived_at: ActiveValue::NotSet,
        };
        history
            .insert(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)?;

        Ok(result)
    }

    async fn get_entity(&self, id: &str) -> kernel::Result<Option<entity_record::Model>> {
        // Since we have a composite primary key (id, tenant_id), we must provide both.
        // RLS will also filter this, but SeaORM requires both for the PK lookup.
        let result =
            EntityRecord::find_by_id((id.to_string(), self.tenant_id.as_str().to_string()))
                .one(&self.inner.txn)
                .await
                .map_err(KernelError::db_error)?;
        Ok(result)
    }

    async fn log_audit(
        &self,
        action: String,
        resource: String,
        details: serde_json::Value,
    ) -> kernel::Result<()> {
        let user_id = self.user_id.as_ref().ok_or_else(|| {
            KernelError::ValidationError("missing actor for audit log".to_string())
        })?;
        let user_id_str = sanitize_for_log(user_id.as_str());

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
        log.insert(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)?;
        Ok(())
    }
}
