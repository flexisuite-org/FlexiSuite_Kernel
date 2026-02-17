use super::connection::{RawConnection, TenantScoped};
use crate::entities::audit_log;
use crate::entities::entity_history;
use crate::entities::entity_record;
use crate::error::DataError;
use async_trait::async_trait;
use sea_orm::{ActiveModelTrait, ActiveValue, EntityTrait};
use crate::entities::prelude::*;
use uuid::Uuid;

/// Sealed trait to prevent external implementations.
pub(crate) mod private {
    pub trait Sealed {}
}

/// The public interface for tenant-scoped database operations.
/// This trait is sealed and can only be implemented within this crate.
#[async_trait]
pub trait TenantRepository: private::Sealed + Send + Sync {
    async fn create_entity(&self, active_model: entity_record::ActiveModel) -> Result<entity_record::Model, DataError>;
    async fn update_entity(&self, active_model: entity_record::ActiveModel) -> Result<entity_record::Model, DataError>;
    async fn get_entity(&self, id: &str) -> Result<Option<entity_record::Model>, DataError>;
    async fn log_audit(
        &self,
        action: String,
        resource: String,
        details: serde_json::Value,
    ) -> Result<(), DataError>;
}

fn pseudonymize_user_id_for_audit(tenant_id: &str, user_id: &str) -> String {
    let scoped = format!("{tenant_id}:{user_id}");
    let digest = ring::digest::digest(&ring::digest::SHA256, scoped.as_bytes());
    format!("uidh:{}", hex::encode(digest.as_ref()))
}

fn history_actor_id(tenant_id: &str, user_id: Option<&crate::auth_context::UserId>) -> String {
    user_id
        .map(|u| pseudonymize_user_id_for_audit(tenant_id, u.as_str()))
        .unwrap_or_else(|| "system".to_string())
}

fn required_active_value<T: Clone>(value: &ActiveValue<T>, field_name: &str) -> Result<T, DataError>
where
    T: Into<sea_orm::Value>,
{
    match value {
        ActiveValue::Set(v) | ActiveValue::Unchanged(v) => Ok(v.clone()),
        ActiveValue::NotSet => Err(DataError::ValidationError(format!(
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
    ) -> Result<entity_record::Model, DataError> {
        // Enforce tenant scoping by overriding the tenant_id field
        active_model.tenant_id = ActiveValue::Set(self.tenant_id.to_string());

        // Ensure ID is set
        let entity_id = required_active_value(&active_model.id, "Entity ID")?;
        let entity_type = required_active_value(&active_model.entity_type, "Entity Type")?;
        let content = required_active_value(&active_model.content, "Content")?;
        let version = active_value_or(&active_model.version, 1);

        let user_id_str = history_actor_id(self.tenant_id.as_str(), self.user_id.as_ref());

        // 1. Insert Entity
        let result = active_model
            .insert(&self.inner.txn)
            .await
            .map_err(DataError::DbError)?;

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
            .map_err(DataError::DbError)?;

        Ok(result)
    }

    async fn update_entity(
        &self,
        mut active_model: entity_record::ActiveModel,
    ) -> Result<entity_record::Model, DataError> {
        // Enforce tenant scoping
        active_model.tenant_id = ActiveValue::Unchanged(self.tenant_id.to_string());

        let entity_id = required_active_value(&active_model.id, "Entity ID")?;

        let existing = EntityRecord::find_by_id((entity_id.clone(), self.tenant_id.to_string()))
            .one(&self.inner.txn)
            .await
            .map_err(DataError::DbError)?
            .ok_or_else(|| DataError::EntityNotFound(format!("Entity {} not found", entity_id)))?;

        match active_model.version.clone() {
            ActiveValue::Set(v) | ActiveValue::Unchanged(v) if v != existing.version => {
                return Err(DataError::ValidationError(format!(
                    "version conflict: expected {}, got {}",
                    existing.version, v
                )));
            }
            _ => {}
        }

        let next_version = existing.version + 1;
        active_model.version = ActiveValue::Set(next_version);
        active_model.updated_at = ActiveValue::Set(chrono::Utc::now().into());

        let user_id_str = history_actor_id(self.tenant_id.as_str(), self.user_id.as_ref());

        // 1. Update Entity
        let result = active_model
            .update(&self.inner.txn)
            .await
            .map_err(DataError::DbError)?;

        let patch = json_patch::diff(&existing.content, &result.content);
        let patch_json = serde_json::to_value(&patch).map_err(|e| {
            DataError::SerializationError(format!("failed to serialize content patch: {e}"))
        })?;

        // 2. Insert History
        let history = entity_history::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7().to_string()),
            tenant_id: ActiveValue::Set(self.tenant_id.to_string()),
            entity_id: ActiveValue::Set(entity_id),
            entity_type: ActiveValue::Set(result.entity_type.clone()),
            change_type: ActiveValue::Set("UPDATE".to_string()),
            version: ActiveValue::Set(next_version),
            diff: ActiveValue::Set(patch_json),
            created_at: ActiveValue::Set(chrono::Utc::now().into()),
            created_by: ActiveValue::Set(user_id_str),
            archived_at: ActiveValue::NotSet,
        };
        history
            .insert(&self.inner.txn)
            .await
            .map_err(DataError::DbError)?;

        Ok(result)
    }

    async fn get_entity(&self, id: &str) -> Result<Option<entity_record::Model>, DataError> {
        // Since we have a composite primary key (id, tenant_id), we must provide both.
        // RLS will also filter this, but SeaORM requires both for the PK lookup.
        let result = EntityRecord::find_by_id((id.to_string(), self.tenant_id.as_str().to_string()))
            .one(&self.inner.txn)
            .await
            .map_err(DataError::DbError)?;
        Ok(result)
    }

    async fn log_audit(
        &self,
        action: String,
        resource: String,
        details: serde_json::Value,
    ) -> Result<(), DataError> {
        let user_id = self.user_id.clone().ok_or_else(|| {
            DataError::ValidationError("missing actor for audit log".to_string())
        })?;
        let user_id_str = pseudonymize_user_id_for_audit(self.tenant_id.as_str(), user_id.as_str());

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
            .map_err(DataError::DbError)?;
        Ok(())
    }
}
