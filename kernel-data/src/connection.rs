use kernel_core::auth::TenantContext;
use kernel_core::kernel::{self, KernelError};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr,
    Statement, TransactionTrait,
};
use tracing::{error, warn};
use uuid::Uuid;

// Sealed Internal Wrapper
pub struct RawConnection(pub(crate) DatabaseTransaction);

/// A wrapper around a database transaction that is guaranteed to be scoped to a specific tenant.
pub struct TenantScoped<C> {
    pub(crate) inner: C,
    pub(crate) tenant_id: String,
}

impl<C> TenantScoped<C> {
    pub(crate) fn new(inner: C, tenant_id: String) -> Self {
        Self { inner, tenant_id }
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

// TenantRepository implementation is in repository.rs

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

    // 1. Set Token
    // Format: v2:kid:ts:nonce:tenant_id:sig
    let now = chrono::Utc::now().timestamp();
    let token = format!(
        "v2:master:{}:{}:{}:mock_sig",
        now,             // ts
        Uuid::now_v7().to_string(), // nonce (unique per call)
        ctx.tenant_id,   // tenant_id
    );

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

    let scoped = TenantScoped::new(RawConnection(txn), ctx.tenant_id.to_string());

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
