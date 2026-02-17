use crate::error::RegistryError;
use crate::model::{Dependencies, DistManifest, Kind, Route};
use bytes::Bytes;
use kernel_core::auth::TenantContext;
use object_store::ObjectStore;
use object_store::path::Path;
use serde::Serialize;
use sha2::{Digest, Sha384};
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct RegistryStorage {
    store: Arc<dyn ObjectStore>,
    prefix: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestDigestPayload<'a> {
    schema_version: &'a str,
    id: &'a str,
    version: &'a str,
    kind: &'a Kind,
    name: &'a str,
    protected: bool,
    composition_root: &'a str,
    routes: &'a [Route],
    dependencies: &'a Dependencies,
    configuration: &'a BTreeMap<String, serde_json::Value>,
}

impl RegistryStorage {
    pub fn new(store: Arc<dyn ObjectStore>, tenant_ctx: &TenantContext) -> Self {
        Self {
            store,
            prefix: format!("tenants/{}/", tenant_ctx.tenant_id().as_str()),
        }
    }

    fn validate_key(key: &str) -> Result<(), RegistryError> {
        if key.is_empty() {
            return Err(RegistryError::InvalidPath(
                "key must not be empty".to_string(),
            ));
        }
        if key.chars().any(char::is_control) {
            return Err(RegistryError::InvalidPath(format!(
                "invalid key contains control character: {key}"
            )));
        }
        if key.contains('\\') {
            return Err(RegistryError::InvalidPath(format!(
                "invalid key contains backslash: {key}"
            )));
        }
        let lower = key.to_ascii_lowercase();
        if lower.contains("%2f") || lower.contains("%5c") {
            return Err(RegistryError::InvalidPath(format!(
                "invalid key contains encoded path separator: {key}"
            )));
        }
        for segment in key.split('/') {
            if segment.is_empty() {
                return Err(RegistryError::InvalidPath(format!(
                    "invalid key contains empty segment: {key}"
                )));
            }
            if segment == "." || segment == ".." {
                return Err(RegistryError::InvalidPath(format!(
                    "invalid key contains traversal segment: {key}"
                )));
            }
        }
        Ok(())
    }

    fn artifact_path(&self, key: &str) -> Path {
        Path::from(format!("{}artifacts/{}", self.prefix, key))
    }

    fn manifest_path(&self, id: &str, version: &str) -> Path {
        Path::from(format!(
            "{}manifests/{}/{}/manifest.json",
            self.prefix, id, version
        ))
    }

    fn manifest_payload_digest(manifest: &DistManifest) -> Result<String, RegistryError> {
        let payload = ManifestDigestPayload {
            schema_version: &manifest.schema_version,
            id: &manifest.id,
            version: &manifest.version,
            kind: &manifest.kind,
            name: &manifest.name,
            protected: manifest.protected,
            composition_root: &manifest.composition_root,
            routes: &manifest.routes,
            dependencies: &manifest.dependencies,
            configuration: &manifest.configuration,
        };
        let payload_bytes = serde_json::to_vec(&payload)?;
        let mut hasher = Sha384::new();
        hasher.update(payload_bytes);
        Ok(hex::encode(hasher.finalize()))
    }

    /// Saves binary data and returns the SHA-384 digest (hex string).
    pub async fn save_artifact(&self, key: &str, data: Bytes) -> Result<String, RegistryError> {
        Self::validate_key(key)?;
        let mut hasher = Sha384::new();
        hasher.update(&data);
        let digest = hex::encode(hasher.finalize());

        let path = self.artifact_path(key);
        self.store.put(&path, data.into()).await?;

        Ok(digest)
    }

    /// Retrieves binary data. If expected_digest is provided, verifies SHA-384.
    pub async fn get_artifact(
        &self,
        key: &str,
        expected_digest: Option<&str>,
    ) -> Result<Bytes, RegistryError> {
        Self::validate_key(key)?;
        let path = self.artifact_path(key);
        let result = self.store.get(&path).await.map_err(|e| match e {
            object_store::Error::NotFound { .. } => {
                RegistryError::ArtifactNotFound(key.to_string())
            }
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
    /// Returns the SHA-384 digest and persisted manifest with manifest_digest set.
    pub async fn save_manifest(
        &self,
        manifest: &DistManifest,
    ) -> Result<(String, DistManifest), RegistryError> {
        Self::validate_key(&manifest.id)?;
        Self::validate_key(&manifest.version)?;
        if manifest.security.manifest_signature.trim().is_empty() {
            return Err(RegistryError::InvalidManifest(
                "security.manifest_signature must not be empty".to_string(),
            ));
        }
        if manifest.security.manifest_signature_kid.trim().is_empty() {
            return Err(RegistryError::InvalidManifest(
                "security.manifest_signature_kid must not be empty".to_string(),
            ));
        }
        if manifest.security.trust_root_version.trim().is_empty() {
            return Err(RegistryError::InvalidManifest(
                "security.trust_root_version must not be empty".to_string(),
            ));
        }

        // manifest_digest is computed from the manifest with the entire
        // security section excluded from the hashed payload.
        let computed_digest = Self::manifest_payload_digest(manifest)?;
        let mut persisted = manifest.clone();
        persisted.security.manifest_digest = computed_digest.clone();

        let path = self.manifest_path(&manifest.id, &manifest.version);
        let data = serde_json::to_vec(&persisted)?;
        self.store.put(&path, data.into()).await?;
        Ok((computed_digest, persisted))
    }

    /// Retrieves a DistManifest from `manifests/{id}/{version}/manifest.json`.
    pub async fn get_manifest(
        &self,
        id: &str,
        version: &str,
    ) -> Result<DistManifest, RegistryError> {
        Self::validate_key(id)?;
        Self::validate_key(version)?;
        let path = self.manifest_path(id, version);
        let result = self.store.get(&path).await.map_err(|e| match e {
            object_store::Error::NotFound { .. } => {
                RegistryError::ManifestNotFound(format!("{id}/{version}"))
            }
            _ => RegistryError::ObjectStore(e),
        })?;
        let data = result.bytes().await?;
        let manifest: DistManifest = serde_json::from_slice(&data)?;
        let actual = Self::manifest_payload_digest(&manifest)?;
        let expected = manifest.security.manifest_digest.clone();
        if actual != expected {
            return Err(RegistryError::IntegrityCheckFailed { expected, actual });
        }
        Ok(manifest)
    }
}
