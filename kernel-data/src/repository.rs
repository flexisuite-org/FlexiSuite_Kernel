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
    async fn update_entity(&self, id: &str, active_model: entity_record::ActiveModel) -> kernel::Result<entity_record::Model>;
    async fn delete_entity(&self, id: &str) -> kernel::Result<()>;
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

    async fn update_entity(&self, id: &str, mut active_model: entity_record::ActiveModel) -> kernel::Result<entity_record::Model> {
        // Enforce tenant scoping by overriding the tenant_id field
        active_model.tenant_id = sea_orm::ActiveValue::Set(self.tenant_id.to_string());
        // Ensure ID matches
        active_model.id = sea_orm::ActiveValue::Set(id.to_string());

        let result = active_model.update(&self.inner.txn).await.map_err(KernelError::db_error)?;
        Ok(result)
    }

    async fn delete_entity(&self, id: &str) -> kernel::Result<()> {
        entity_record::Entity::delete_by_id((id.to_string(), self.tenant_id.to_string()))
            .exec(&self.inner.txn)
            .await
            .map_err(KernelError::db_error)?;
        Ok(())
    }
}
