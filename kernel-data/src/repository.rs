use super::connection::{RawConnection, TenantScoped};
use crate::entities::prelude::*;
use crate::entities::{entity_record, diagnostic_report, diagnostic_policy};
use async_trait::async_trait;
use kernel_core::kernel::{self, KernelError};
use sea_orm::{ActiveModelTrait, EntityTrait, ColumnTrait, QueryFilter};
use sea_orm::sea_query::OnConflict;

/// Sealed trait to prevent external implementations.
pub(crate) mod private {
    pub trait Sealed {}
}

/// The public interface for tenant-scoped database operations.
/// This trait is sealed and can only be implemented within this crate.
#[async_trait]
pub trait TenantRepository: private::Sealed + Send + Sync {
    async fn create_entity(&self, active_model: entity_record::ActiveModel) -> kernel::Result<entity_record::Model>;
    async fn get_entity(&self, id: &str) -> kernel::Result<Option<entity_record::Model>>;

    // Diagnostic methods
    async fn create_diagnostic_report(&self, active_model: diagnostic_report::ActiveModel) -> kernel::Result<diagnostic_report::Model>;
    async fn get_diagnostic_report(&self, trace_id: &str) -> kernel::Result<Option<diagnostic_report::Model>>;
    async fn get_diagnostic_policy(&self) -> kernel::Result<Option<diagnostic_policy::Model>>;
    async fn upsert_diagnostic_policy(&self, active_model: diagnostic_policy::ActiveModel) -> kernel::Result<diagnostic_policy::Model>;
}

#[async_trait]
impl TenantRepository for TenantScoped<RawConnection> {
    async fn create_entity(&self, mut active_model: entity_record::ActiveModel) -> kernel::Result<entity_record::Model> {
        // Enforce tenant scoping by overriding the tenant_id field
        active_model.tenant_id = sea_orm::ActiveValue::Set(self.tenant_id.to_string());
        
        let result = active_model.insert(&self.inner.txn).await.map_err(KernelError::db_error)?;
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

    async fn create_diagnostic_report(&self, mut active_model: diagnostic_report::ActiveModel) -> kernel::Result<diagnostic_report::Model> {
        active_model.tenant_id = sea_orm::ActiveValue::Set(self.tenant_id.to_string());
        let result = active_model.insert(&self.inner.txn).await.map_err(KernelError::db_error)?;
        Ok(result)
    }

    async fn get_diagnostic_report(&self, trace_id: &str) -> kernel::Result<Option<diagnostic_report::Model>> {
        let result = DiagnosticReport::find_by_id(trace_id.to_string())
            .filter(diagnostic_report::Column::TenantId.eq(self.tenant_id.as_str()))
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

    async fn upsert_diagnostic_policy(&self, mut active_model: diagnostic_policy::ActiveModel) -> kernel::Result<diagnostic_policy::Model> {
        active_model.tenant_id = sea_orm::ActiveValue::Set(self.tenant_id.to_string());

        let result = DiagnosticPolicy::insert(active_model)
            .on_conflict(
                OnConflict::column(diagnostic_policy::Column::TenantId)
                    .update_columns([
                        diagnostic_policy::Column::Enabled,
                        diagnostic_policy::Column::UpdatedAt,
                        diagnostic_policy::Column::UpdatedBy
                    ])
                    .to_owned()
            )
            .exec_with_returning(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)?;

        Ok(result)
    }
}
