use thiserror::Error;

#[derive(Error, Debug)]
pub enum KernelError {
    #[error("Database error: {0}")]
    DbError(#[from] sea_orm::DbErr),

    #[error("Tenant authorization failed: {0}")]
    TenantAuthorizationFailed(String),

    #[error("Commit failed with unknown outcome: {0}")]
    CommitUnknown(String),

    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("Validation error: {0}")]
    ValidationError(String),
}

pub type Result<T> = std::result::Result<T, KernelError>;
