use thiserror::Error;

pub type Result<T> = std::result::Result<T, KernelError>;

#[derive(Debug, Error)]
pub enum KernelError {
    #[error("Database error: {0}")]
    DbError(#[from] sea_orm::DbErr),

    #[error("Data error: {0}")]
    DataError(#[from] kernel_data::DataError),

    #[error("Tenant authorization failed: {0}")]
    TenantAuthorizationFailed(String),

    #[error("Commit failed: {0}")]
    CommitUnknown(String),
    // Other errors...
}

// Helper to convert DataError to KernelError (impl From is derived)
