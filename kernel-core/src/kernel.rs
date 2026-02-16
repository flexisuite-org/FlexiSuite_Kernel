use thiserror::Error;

#[derive(Error, Debug)]
pub enum KernelError {
    #[error("Database error: {0}")]
    DbError(String),

    #[error("Tenant authorization failed: {0}")]
    TenantAuthorizationFailed(String),

    #[error("Commit failed with unknown outcome: {0}")]
    CommitUnknown(String),

    /// Used for features that are planned but not yet implemented.
    /// In production, this should be avoided or gated behind feature flags.
    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("Validation error: {0}")]
    ValidationError(String),
}

impl KernelError {
    pub fn db_error<E: std::fmt::Display>(e: E) -> Self {
        Self::DbError(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, KernelError>;
