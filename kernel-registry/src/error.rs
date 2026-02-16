use thiserror::Error;

#[derive(Error, Debug)]
pub enum RegistryError {
    #[error("Object store error: {0}")]
    ObjectStore(#[from] object_store::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Integrity check failed: expected {expected}, got {actual}")]
    IntegrityCheckFailed { expected: String, actual: String },

    #[error("Manifest not found: {0}")]
    ManifestNotFound(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),
}
