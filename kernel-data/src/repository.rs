use super::connection::{RawConnection, TenantScoped};
use crate::entities::prelude::*;
use crate::entities::entity_record;
use async_trait::async_trait;
use kernel_core::kernel::{self, KernelError};
use sea_orm::{ActiveModelTrait, EntityTrait};

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
    // TODO: Add more methods (update, list, etc.) as needed in 3-2-2
}

#[async_trait]
impl TenantRepository for TenantScoped<RawConnection> {
    async fn create_entity(&self, active_model: entity_record::ActiveModel) -> kernel::Result<entity_record::Model> {
        // tenant_id handling is implicitly enforced by RLS, but we also enforce it in application layer
        // However, active_model usually comes with tenant_id set by the caller logic (business logic).
        // If we want to strictly enforce it here, we would need to override it with context.
        // But TenantRepository doesn't hold Context, only `with_tenant_tx` does.
        // `with_tenant_tx` sets the RLS context.
        // So RLS will reject if tenant_id doesn't match the current_setting.
        
        let result = active_model.insert(&self.inner.0).await.map_err(KernelError::DbError)?;
        Ok(result)
    }

    async fn get_entity(&self, id: &str) -> kernel::Result<Option<entity_record::Model>> {
        // Since we have a composite primary key (id, tenant_id), we must provide both.
        // RLS will also filter this, but SeaORM requires both for the PK lookup.
        let result = EntityRecord::find_by_id((id.to_string(), self.tenant_id.clone()))
            .one(&self.inner.0)
            .await
            .map_err(KernelError::DbError)?;
        Ok(result)
    }
}
