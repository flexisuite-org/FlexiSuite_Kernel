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

    #[error("Artifact not found: {0}")]
    ArtifactNotFound(String),

    #[error("Artifact exceeds maximum allowed size: {actual} bytes (max {max} bytes)")]
    ArtifactTooLarge { max: usize, actual: usize },

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Invalid manifest: {0}")]
    InvalidManifest(String),

    #[error("Key is revoked and cannot be used: {kid}")]
    KeyRevoked { kid: String },

    #[error("Key is not yet valid: {kid} (not_before={valid_from}, now={now})")]
    KeyNotYetValid {
        kid: String,
        valid_from: u64,
        now: u64,
    },

    #[error("Key has expired: {kid} (not_after={expired_at}, now={now})")]
    KeyExpired {
        kid: String,
        expired_at: u64,
        now: u64,
    },

    #[error("Trust root error: {0}")]
    TrustRootError(String),
}
