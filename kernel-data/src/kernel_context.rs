use sea_orm::{ActiveValue, ConnectionTrait, DatabaseConnection, DatabaseTransaction, EntityTrait, QueryTrait, TransactionTrait};
use std::sync::Arc;
use uuid::Uuid;
use crate::entities::audit_log;

/// A marker token that can only be constructed by the background task runner.
/// This prevents API handlers from constructing a `KernelContext` directly.
#[derive(Clone, Debug)]
pub struct BackgroundRunnerToken(());

impl BackgroundRunnerToken {
    /// Constructs a `BackgroundRunnerToken`.
    /// This is strictly limited to background workers and task runners.
    /// It avoids exposing the constructor publicly to prevent API handlers
    /// from accidentally or maliciously constructing a `KernelContext`.
    #[allow(dead_code)]
    pub(crate) fn new() -> Self {
        Self(())
    }
}

/// Provides a controlled entry point for explicitly designated background runners
/// (e.g., `kernel-archiver` or `kernel-api` background tasks) to construct a `KernelContext`.
///
/// Do NOT use this in request-scoped API handlers.
/// Any usage must be carefully reviewed to ensure it runs as a background process.
#[cfg(feature = "background_worker")]
pub fn create_background_runner_context(db: Arc<DatabaseConnection>) -> KernelContext {
    let token = BackgroundRunnerToken::new();
    KernelContext::new(token, db)
}

/// A context for performing cross-tenant, privileged operations.
///
/// It should be initialized with a database connection pool that uses the
/// `flexi_kernel_admin` role. Operations using this context rely on
/// `SECURITY DEFINER` functions in the database to safely bypass RLS
/// without compromising tenant isolation.
#[derive(Clone, Debug)]
pub struct KernelContext {
    db: Arc<DatabaseConnection>,
}

impl KernelContext {
    /// Constructs a new `KernelContext`.
    /// Requires a `BackgroundRunnerToken` to ensure it is not called from
    /// normal API handlers.
    pub fn new(_token: BackgroundRunnerToken, db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }

    /// Accesses the underlying database connection.
    pub fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    /// Executes a closure within a database transaction.
    ///
    /// The closure should execute specific `SECURITY DEFINER` functions
    /// using `flexi_kernel_admin` privileges.
    pub async fn with_tx<F, R>(&self, f: F) -> Result<R, sea_orm::DbErr>
    where
        F: for<'c> FnOnce(
            &'c DatabaseTransaction,
        ) -> futures::future::BoxFuture<'c, Result<R, sea_orm::DbErr>>,
    {
        let txn = self.db.begin().await?;
        match f(&txn).await {
            Ok(result) => {
                txn.commit().await?;
                Ok(result)
            }
            Err(e) => {
                let _ = txn.rollback().await;
                Err(e)
            }
        }
    }

    /// Logs an audit record for a privileged cross-tenant operation.
    ///
    /// Because `KernelContext` operates across tenants or operates on system-level
    /// maintenance tasks, it uses a generic `system` tenant context in the audit log
    /// and ensures the background operation has a trace.
    pub async fn log_privileged_audit(
        &self,
        txn: &DatabaseTransaction,
        action: String,
        resource: String,
        details: serde_json::Value,
    ) -> Result<(), sea_orm::DbErr> {
        let log = audit_log::ActiveModel {
            id: ActiveValue::Set(Uuid::now_v7().to_string()),
            tenant_id: ActiveValue::Set("system".to_string()),
            actor_id: ActiveValue::Set("kernel_admin".to_string()),
            action: ActiveValue::Set(action),
            resource: ActiveValue::Set(resource),
            details: ActiveValue::Set(details),
            ip_address: ActiveValue::NotSet,
            user_agent: ActiveValue::Set(Some("kernel-background-runner".to_string())),
            created_at: ActiveValue::Set(chrono::Utc::now().into()),
            archived_at: ActiveValue::NotSet,
        };

        let stmt = audit_log::Entity::insert(log).build(txn.get_database_backend());
        txn.execute(stmt).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult, TransactionTrait};
    use serde_json::json;

    #[tokio::test]
    async fn test_kernel_context_with_tx_success() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results([
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 0,
                }, // begin
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                }, // insert
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 0,
                }, // commit
            ])
            .into_connection();

        let db = Arc::new(db);
        let ctx = KernelContext::new(BackgroundRunnerToken::new(), db);

        let result = ctx
            .with_tx(|_txn| {
                Box::pin(async move {
                    // Simulate some work using `txn`
                    // Under normal circumstances, this would call a SECURITY DEFINER function.
                    Ok(42)
                })
            })
            .await;

        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_kernel_context_log_privileged_audit() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results([
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 0,
                }, // begin
            ])
            .append_query_results([vec![
                audit_log::Model {
                    id: Uuid::now_v7().to_string(),
                    tenant_id: "system".to_string(),
                    actor_id: "kernel_admin".to_string(),
                    action: "test_action".to_string(),
                    resource: "test_resource".to_string(),
                    details: json!({"key": "value"}),
                    ip_address: None,
                    user_agent: Some("kernel-background-runner".to_string()),
                    created_at: chrono::Utc::now().into(),
                    archived_at: None,
                }
            ]])
            .append_exec_results([
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                }, // insert fallback if returning isn't mapped
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 0,
                }, // commit
            ])
            .into_connection();

        let db = Arc::new(db);
        let ctx = KernelContext::new(BackgroundRunnerToken::new(), db);

        let ctx_clone = ctx.clone();
        let result = ctx
            .with_tx(|txn| {
                let ctx_inner = ctx_clone.clone();
                Box::pin(async move {
                    ctx_inner.log_privileged_audit(
                        txn,
                        "test_action".to_string(),
                        "test_resource".to_string(),
                        json!({"key": "value"}),
                    )
                    .await?;
                    Ok(())
                })
            })
            .await;

        if let Err(e) = &result {
            println!("Error: {:?}", e);
        }
        assert!(result.is_ok());
    }
}
