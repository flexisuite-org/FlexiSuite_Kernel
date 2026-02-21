use thiserror::Error;

pub const INVALID_OR_EXPIRED_KEY_ID: &str = "Invalid or expired key ID";

#[derive(Debug, Error)]
pub enum DataError {
    #[error("Database error: {0}")]
    DbError(#[from] sea_orm::DbErr),
    #[error("Tenant authorization failed: {0}")]
    TenantAuthorizationFailed(String),
    #[error("Commit failed (unknown outcome): {0}")]
    CommitUnknown(String),
    #[error("Transaction failed: {0}")]
    TransactionFailed(String),
    #[error("Entity not found: {0}")]
    EntityNotFound(String),
    #[error("Validation error: {0}")]
    ValidationError(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
}
