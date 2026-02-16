use bytes::Bytes;
use object_store::path::Path;
use object_store::ObjectStore;
use sha2::{Digest, Sha384};
use std::sync::Arc;
use crate::error::RegistryError;
use crate::model::DistManifest;

pub struct RegistryStorage {
    store: Arc<dyn ObjectStore>,
    prefix: String,
}

impl RegistryStorage {
    pub fn new(store: Arc<dyn ObjectStore>) -> Self {
        Self {
            store,
            prefix: "artifacts/".to_string(),
        }
    }

    fn get_path(&self, key: &str) -> Path {
        let key = key.trim_start_matches('/');
        // Assuming self.prefix ends with '/'
        Path::from(format!("{}{}", self.prefix, key))
    }

    /// Saves binary data and returns the SHA-384 digest (hex string).
    pub async fn save_artifact(&self, key: &str, data: Bytes) -> Result<String, RegistryError> {
        let mut hasher = Sha384::new();
        hasher.update(&data);
        let digest = hex::encode(hasher.finalize());

        let path = self.get_path(key);
        self.store.put(&path, data.into()).await?;

        Ok(digest)
    }

    /// Retrieves binary data. If expected_digest is provided, verifies SHA-384.
    pub async fn get_artifact(&self, key: &str, expected_digest: Option<&str>) -> Result<Bytes, RegistryError> {
        let path = self.get_path(key);
        let result = self.store.get(&path).await.map_err(|e| match e {
            object_store::Error::NotFound { .. } => RegistryError::ManifestNotFound(key.to_string()),
            _ => RegistryError::ObjectStore(e),
        })?;

        let data = result.bytes().await?;

        if let Some(expected) = expected_digest {
            let mut hasher = Sha384::new();
            hasher.update(&data);
            let actual = hex::encode(hasher.finalize());
            if actual != expected {
                return Err(RegistryError::IntegrityCheckFailed {
                    expected: expected.to_string(),
                    actual,
                });
            }
        }

        Ok(data)
    }

    /// Saves a DistManifest to `manifests/{id}/{version}/manifest.json`.
    /// Returns the SHA-384 digest of the stored manifest.
    pub async fn save_manifest(&self, manifest: &DistManifest) -> Result<String, RegistryError> {
        // Path convention: manifests/{id}/{version}/manifest.json
        // Note: DistManifest should have a `version` field.
        let path_str = format!("manifests/{}/{}/manifest.json", manifest.id, manifest.version);
        let data = serde_json::to_vec(manifest)?;
        self.save_artifact(&path_str, data.into()).await
    }

    /// Retrieves a DistManifest from `manifests/{id}/{version}/manifest.json`.
    pub async fn get_manifest(&self, id: &str, version: &str) -> Result<DistManifest, RegistryError> {
        let path_str = format!("manifests/{id}/{version}/manifest.json");
        // We don't check digest here as we don't know it beforehand unless passed.
        // The registry index would store the digest if needed.
        let data = self.get_artifact(&path_str, None).await?;
        let manifest: DistManifest = serde_json::from_slice(&data)?;
        Ok(manifest)
    }
}
