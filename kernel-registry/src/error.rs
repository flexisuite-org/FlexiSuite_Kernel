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

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Invalid manifest: {0}")]
    InvalidManifest(String),

    #[error("Trust root error: {0}")]
    TrustRootError(String),

    #[error("Signature verification failed: {0}")]
    SignatureVerificationFailed(String),

    #[error("Trust root key not found: {0}")]
    KeyNotFound(String),
}
