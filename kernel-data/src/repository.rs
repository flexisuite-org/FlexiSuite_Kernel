use super::connection::{RawConnection, TenantScoped};
use crate::entities::audit_log;
use crate::entities::entity_history;
use crate::entities::entity_record;
use crate::entities::prelude::*;
use crate::entities::{diagnostic_policy, diagnostic_report};
use async_trait::async_trait;
use kernel_core::kernel::{self, KernelError};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};
use uuid::Uuid;

const MARK_ARCHIVE_CHUNK_SIZE: usize = 500;

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
    async fn get_entity(&self, id: &str) -> kernel::Result<Option<entity_record::Model>>;
    async fn update_entity(
        &self,
        id: &str,
        active_model: entity_record::ActiveModel,
    ) -> kernel::Result<entity_record::Model>;
    async fn delete_entity(&self, id: &str) -> kernel::Result<()>;

    // Diagnostic methods
    async fn create_diagnostic_report(
        &self,
        active_model: diagnostic_report::ActiveModel,
    ) -> kernel::Result<diagnostic_report::Model>;
    async fn get_diagnostic_report(
        &self,
        trace_id: &str,
    ) -> kernel::Result<Option<diagnostic_report::Model>>;
    async fn get_diagnostic_policy(&self) -> kernel::Result<Option<diagnostic_policy::Model>>;
    async fn upsert_diagnostic_policy(
        &self,
        active_model: diagnostic_policy::ActiveModel,
    ) -> kernel::Result<diagnostic_policy::Model>;

    async fn log_audit(
        &self,
        action: String,
        resource: String,
        details: serde_json::Value,
    ) -> kernel::Result<()>;

    // Archive methods (for kernel-archiver)
    async fn find_unarchived_entity_histories(
        &self,
        limit: u64,
    ) -> kernel::Result<Vec<entity_history::Model>>;
    async fn mark_entity_history_archived(&self, id: String) -> kernel::Result<()>;
    async fn mark_entity_histories_archived(&self, ids: Vec<String>) -> kernel::Result<()>;
    async fn find_unarchived_audit_logs(&self, limit: u64)
    -> kernel::Result<Vec<audit_log::Model>>;
    async fn mark_audit_log_archived(&self, id: String) -> kernel::Result<()>;
    async fn mark_audit_logs_archived(&self, ids: Vec<String>) -> kernel::Result<()>;
}

fn pseudonymize_user_id_for_audit(tenant_id: &str, user_id: &str) -> String {
    let scoped = format!("{tenant_id}:{user_id}");
    let digest = ring::digest::digest(&ring::digest::SHA256, scoped.as_bytes());
    format!("uidh:{}", hex::encode(digest.as_ref()))
}

fn history_actor_id(tenant_id: &str, user_id: Option<&kernel_core::auth::UserId>) -> String {
    user_id
        .map(|u| pseudonymize_user_id_for_audit(tenant_id, u.as_str()))
        .unwrap_or_else(|| "system".to_string())
}

fn required_active_value<T: Clone>(value: &ActiveValue<T>, field_name: &str) -> kernel::Result<T>
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

        let user_id_str = history_actor_id(self.tenant_id.as_str(), self.user_id.as_ref());

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

    async fn create_diagnostic_report(
        &self,
        mut active_model: diagnostic_report::ActiveModel,
    ) -> kernel::Result<diagnostic_report::Model> {
        active_model.tenant_id = sea_orm::ActiveValue::Set(self.tenant_id.to_string());
        let result = active_model
            .insert(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)?;
        Ok(result)
    }

    async fn get_diagnostic_report(
        &self,
        trace_id: &str,
    ) -> kernel::Result<Option<diagnostic_report::Model>> {
        let result =
            DiagnosticReport::find_by_id((trace_id.to_string(), self.tenant_id.to_string()))
                .one(&self.inner.txn)
                .await
                .map_err(KernelError::db_error)?;
        Ok(result)
    }

    async fn get_diagnostic_policy(&self) -> kernel::Result<Option<diagnostic_policy::Model>> {
        let result = DiagnosticPolicy::find_by_id(self.tenant_id.to_string())
            .one(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)?;
        Ok(result)
    }

    async fn update_entity(
        &self,
        id: &str,
        mut active_model: entity_record::ActiveModel,
    ) -> kernel::Result<entity_record::Model> {
        // Enforce tenant scoping
        active_model.tenant_id = ActiveValue::Unchanged(self.tenant_id.to_string());

        match active_model.id.clone() {
            ActiveValue::Set(model_id) | ActiveValue::Unchanged(model_id) if model_id != id => {
                return Err(KernelError::ValidationError(format!(
                    "entity id mismatch: path={id}, payload={model_id}"
                )));
            }
            _ => {}
        }
        active_model.id = ActiveValue::Unchanged(id.to_string());

        let existing = EntityRecord::find_by_id((id.to_string(), self.tenant_id.to_string()))
            .one(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)?
            .ok_or_else(|| KernelError::ValidationError("Entity not found".into()))?;

        match active_model.version.clone() {
            ActiveValue::Set(v) | ActiveValue::Unchanged(v) if v != existing.version => {
                return Err(KernelError::ValidationError(format!(
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
            .map_err(KernelError::db_error)?;

        let patch = json_patch::diff(&existing.content, &result.content);
        let patch_json = serde_json::to_value(&patch).map_err(|e| {
            KernelError::ValidationError(format!("failed to serialize content patch: {e}"))
        })?;

        // 2. Insert History
        let history = entity_history::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7().to_string()),
            tenant_id: ActiveValue::Set(self.tenant_id.to_string()),
            entity_id: ActiveValue::Set(id.to_string()),
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
            .map_err(KernelError::db_error)?;

        Ok(result)
    }

    async fn delete_entity(&self, id: &str) -> kernel::Result<()> {
        let delete_result =
            entity_record::Entity::delete_by_id((id.to_string(), self.tenant_id.to_string()))
                .exec(&self.inner.txn)
                .await
                .map_err(KernelError::db_error)?;
        if delete_result.rows_affected == 0 {
            return Err(KernelError::ValidationError("Entity not found".into()));
        }
        Ok(())
    }

    async fn upsert_diagnostic_policy(
        &self,
        mut active_model: diagnostic_policy::ActiveModel,
    ) -> kernel::Result<diagnostic_policy::Model> {
        active_model.tenant_id = sea_orm::ActiveValue::Set(self.tenant_id.to_string());

        let result = DiagnosticPolicy::insert(active_model)
            .on_conflict(
                OnConflict::column(diagnostic_policy::Column::TenantId)
                    .update_columns([
                        diagnostic_policy::Column::Enabled,
                        diagnostic_policy::Column::UpdatedAt,
                        diagnostic_policy::Column::UpdatedBy,
                    ])
                    .to_owned(),
            )
            .exec_with_returning(&self.inner.txn)
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
        let user_id = self.user_id.clone().ok_or_else(|| {
            KernelError::ValidationError("missing actor for audit log".to_string())
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
            .map_err(KernelError::db_error)?;
        Ok(())
    }

    async fn find_unarchived_entity_histories(
        &self,
        limit: u64,
    ) -> kernel::Result<Vec<entity_history::Model>> {
        EntityHistory::find()
            .filter(entity_history::Column::TenantId.eq(self.tenant_id.to_string()))
            .filter(entity_history::Column::ArchivedAt.is_null())
            .order_by_asc(entity_history::Column::CreatedAt)
            .limit(limit)
            .all(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)
    }

    async fn mark_entity_history_archived(&self, id: String) -> kernel::Result<()> {
        self.mark_entity_histories_archived(vec![id]).await
    }

    async fn mark_entity_histories_archived(&self, ids: Vec<String>) -> kernel::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let tenant_id = self.tenant_id.to_string();
        for chunk in ids.chunks(MARK_ARCHIVE_CHUNK_SIZE) {
            EntityHistory::update_many()
                .col_expr(
                    entity_history::Column::ArchivedAt,
                    Expr::current_timestamp().into(),
                )
                .filter(entity_history::Column::Id.is_in(chunk.iter().cloned()))
                .filter(entity_history::Column::TenantId.eq(tenant_id.clone()))
                .filter(entity_history::Column::ArchivedAt.is_null())
                .exec(&self.inner.txn)
                .await
                .map_err(KernelError::db_error)?;
        }
        Ok(())
    }

    async fn find_unarchived_audit_logs(
        &self,
        limit: u64,
    ) -> kernel::Result<Vec<audit_log::Model>> {
        AuditLog::find()
            .filter(audit_log::Column::TenantId.eq(self.tenant_id.to_string()))
            .filter(audit_log::Column::ArchivedAt.is_null())
            .order_by_asc(audit_log::Column::CreatedAt)
            .limit(limit)
            .all(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)
    }

    async fn mark_audit_log_archived(&self, id: String) -> kernel::Result<()> {
        self.mark_audit_logs_archived(vec![id]).await
    }

    async fn mark_audit_logs_archived(&self, ids: Vec<String>) -> kernel::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let tenant_id = self.tenant_id.to_string();
        for chunk in ids.chunks(MARK_ARCHIVE_CHUNK_SIZE) {
            AuditLog::update_many()
                .col_expr(
                    audit_log::Column::ArchivedAt,
                    Expr::current_timestamp().into(),
                )
                .filter(audit_log::Column::Id.is_in(chunk.iter().cloned()))
                .filter(audit_log::Column::TenantId.eq(tenant_id.clone()))
                .filter(audit_log::Column::ArchivedAt.is_null())
                .exec(&self.inner.txn)
                .await
                .map_err(KernelError::db_error)?;
        }
        Ok(())
    }
}
