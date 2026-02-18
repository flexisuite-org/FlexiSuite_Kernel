use futures::future::BoxFuture;
use kernel_core::auth::TenantContext;
use kernel_core::kernel::{self, KernelError};
use ring::hmac;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr, Statement,
    TransactionTrait,
};
use tracing::{error, warn};
use uuid::Uuid;

use std::sync::OnceLock;

// HMAC Secret Management
static HMAC_SECRET: OnceLock<Vec<u8>> = OnceLock::new();

pub fn init_hmac_secret() -> Result<(), String> {
    let secret = std::env::var("FLEXI_HMAC_SECRET")
        .map_err(|_| "FLEXI_HMAC_SECRET is not set".to_string())?;

    init_hmac_secret_from_string(secret)
}

#[cfg(feature = "test-utils")]
pub fn init_hmac_secret_for_test(secret: impl Into<String>) -> Result<(), String> {
    init_hmac_secret_from_string(secret.into())
}

fn init_hmac_secret_from_string(secret: String) -> Result<(), String> {
    if secret.is_empty() {
        return Err("FLEXI_HMAC_SECRET cannot be empty".to_string());
    }

    if secret.as_bytes().len() < 32 {
        return Err("FLEXI_HMAC_SECRET must be at least 32 bytes".to_string());
    }

    HMAC_SECRET
        .set(secret.into_bytes())
        .map_err(|_| "HMAC secret already initialized".to_string())
}

fn get_hmac_secret() -> Result<&'static [u8], KernelError> {
    HMAC_SECRET.get().map(|s| s.as_slice()).ok_or_else(|| {
        error!("HMAC secret not initialized");
        KernelError::TenantAuthorizationFailed("HMAC secret not initialized".to_string())
    })
}

// Sealed Internal Wrapper
pub struct RawConnection {
    pub(crate) txn: DatabaseTransaction,
}

impl RawConnection {
    pub(crate) fn new(txn: DatabaseTransaction) -> Self {
        Self { txn }
    }
}

/// A wrapper around a database transaction that is guaranteed to be scoped to a specific tenant.
pub struct TenantScoped<C> {
    pub(crate) inner: C,
    pub(crate) tenant_id: kernel_core::auth::TenantId,
    pub(crate) user_id: Option<kernel_core::auth::UserId>,
}

impl<C> TenantScoped<C> {
    pub(super) fn new(
        inner: C,
        tenant_id: kernel_core::auth::TenantId,
        user_id: Option<kernel_core::auth::UserId>,
    ) -> Self {
        Self {
            inner,
            tenant_id,
            user_id,
        }
    }
}

impl TenantScoped<RawConnection> {
    /// Access the underlying database transaction.
    ///
    /// # Visibility
    /// Restricted to `pub(crate)` to prevent external callers from bypassing
    /// tenant isolation. External crates (e.g., kernel-archiver) should use
    /// `TenantRepository` trait methods instead.
    #[allow(dead_code)]
    pub(crate) fn txn(&self) -> &DatabaseTransaction {
        &self.inner.txn
    }

    pub(crate) async fn commit(self) -> Result<(), DbErr> {
        self.inner.txn.commit().await
    }

    pub(crate) async fn rollback(self) -> Result<(), DbErr> {
        self.inner.txn.rollback().await
    }
}

// Implement Sealed trait for TenantScoped
impl super::repository::private::Sealed for TenantScoped<RawConnection> {}

// TenantRepository implementation is in repository.rs

/// Executes a closure within a tenant-scoped transaction.
///
/// **Note:** This function relies on the `flexi.authorize_tenant` PL/pgSQL function, which
/// requires the `flexi.hmac_secret` GUC to be set in the database (e.g., via `ALTER DATABASE ... SET ...`).
/// If the secret is not set, authorization will fail.
pub async fn with_tenant_tx<F, R>(
    pool: &DatabaseConnection,
    ctx: &TenantContext,
    f: F,
) -> kernel::Result<R>
where
    F: for<'c> FnOnce(&'c TenantScoped<RawConnection>) -> BoxFuture<'c, kernel::Result<R>> + Send,
    R: Send,
{
    // Fail fast on configuration/validation errors before acquiring a connection
    if ctx.tenant_id().as_str().contains(':') {
        return Err(KernelError::TenantAuthorizationFailed(
            "tenant_id must not contain ':'".into(),
        ));
    }
    let secret = get_hmac_secret()?;

    let txn = pool.begin().await.map_err(KernelError::db_error)?;

    // 1. Set Token
    // Format: v2:kid:ts:nonce:tenant_id:sig

    let now = chrono::Utc::now().timestamp();
    let ts_str = now.to_string();
    let nonce = Uuid::now_v7().to_string();
    let kid = "master";
    let ver = "v2";

    // HMAC Signature Calculation
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let msg = format!("{}:{}:{}:{}:{}", ver, kid, ts_str, nonce, ctx.tenant_id());
    let tag = hmac::sign(&key, msg.as_bytes());
    let sig = hex::encode(tag.as_ref());

    let token = format!(
        "{}:{}:{}:{}:{}:{}",
        ver,
        kid,
        ts_str,
        nonce,
        ctx.tenant_id(),
        sig
    );

    // 2. Authorize
    txn.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT flexi.authorize_tenant($1)",
        [token.into()],
    ))
    .await
    .map_err(|e| {
        warn!("Tenant authorization failed via DB: {}", e);
        KernelError::TenantAuthorizationFailed(e.to_string())
    })?;

    let scoped = TenantScoped::new(
        RawConnection::new(txn),
        ctx.tenant_id().clone(),
        ctx.user_id().cloned(),
    );

    match f(&scoped).await {
        Ok(result) => match scoped.commit().await {
            Ok(()) => Ok(result),
            Err(commit_err) => {
                error!(
                    tenant_id = %ctx.tenant_id(),
                    "commit failed (outcome unknown): {commit_err}"
                );
                Err(KernelError::CommitUnknown(commit_err.to_string()))
            }
        },
        Err(e) => {
            if let Err(rollback_err) = scoped.rollback().await {
                error!("rollback failed: {rollback_err}");
            }
            warn!(tenant_id = %ctx.tenant_id(), "tx rolled back: {e}");
            Err(e)
        }
    }
}
