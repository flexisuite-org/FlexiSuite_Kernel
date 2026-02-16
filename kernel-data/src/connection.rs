use crate::repository::TenantRepository;
use kernel_api::auth::TenantContext;
use kernel_core::kernel::{self, KernelError};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr,
    Statement, TransactionTrait,
};
use tracing::{warn, error}; // Added error import

// Sealed Internal Wrapper
pub(crate) struct RawConnection(pub(crate) DatabaseTransaction);

/// A wrapper around a database transaction that is guaranteed to be scoped to a specific tenant.
pub struct TenantScoped<C> {
    inner: C,
}

impl<C> TenantScoped<C> {
    pub(crate) fn new(inner: C) -> Self {
        Self { inner }
    }
}

impl TenantScoped<RawConnection> {
    pub(crate) async fn commit(self) -> Result<(), DbErr> {
        self.inner.0.commit().await
    }
    
    pub(crate) async fn rollback(self) -> Result<(), DbErr> {
        self.inner.0.rollback().await
    }
}

// Implement Sealed trait for TenantScoped
impl super::repository::private::Sealed for TenantScoped<RawConnection> {}

#[async_trait::async_trait]
impl TenantRepository for TenantScoped<RawConnection> {
    // Implementation of repository methods will go here
    // Example:
    // async fn find_by_id(&self, id: &str) -> Result<Option<Entity>, KernelError> {
    //     ...
    // }
}

/// Executes a closure within a tenant-scoped transaction.
pub async fn with_tenant_tx<F, R, Fut>(
    pool: &DatabaseConnection,
    ctx: &TenantContext,
    f: F,
) -> kernel::Result<R>
where
    F: FnOnce(&TenantScoped<RawConnection>) -> Fut + Send,
    Fut: std::future::Future<Output = kernel::Result<R>> + Send,
    R: Send,
{
    let txn = pool.begin().await.map_err(KernelError::DbError)?;

    // Mock token generation (Must be replaced with real crypto in future)
    let token = format!("v2:mock_signature:{}:{}", ctx.tenant_id, "nonce");

    // 1. Set Token
    txn.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SET LOCAL flexi.tenant_token = $1",
        [token.into()],
    ))
    .await
    .map_err(KernelError::DbError)?;

    // 2. Authorize
    txn.execute(Statement::from_string(
        DbBackend::Postgres,
        "SELECT flexi.authorize_tenant()".to_owned(),
    ))
    .await
    .map_err(|e| {
        warn!("Tenant authorization failed via DB: {}", e);
        KernelError::TenantAuthorizationFailed(e.to_string())
    })?;

    let scoped = TenantScoped::new(RawConnection(txn));

    match f(&scoped).await {
        Ok(result) => {
            match scoped.commit().await {
                Ok(()) => Ok(result),
                Err(commit_err) => {
                    error!(
                        tenant_id = %ctx.tenant_id,
                        "commit failed (outcome unknown): {commit_err}"
                    );
                    Err(KernelError::CommitUnknown(commit_err.to_string()))
                }
            }
        }
        Err(e) => {
            if let Err(rollback_err) = scoped.rollback().await {
               error!("rollback failed: {rollback_err}");
            }
            warn!(tenant_id = %ctx.tenant_id, "tx rolled back: {e}");
            Err(e)
        }
    }
}
