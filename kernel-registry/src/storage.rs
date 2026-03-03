use crate::error::RegistryError;
use crate::model::{Dependencies, DistManifest, Kind, Route};
use bytes::Bytes;
use futures_util::StreamExt;
use kernel_core::auth::TenantContext;
use object_store::path::Path;
use object_store::{ObjectStore, PutMode, PutOptions};
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::Serialize;
use sha2::{Digest, Sha384};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{error, info, instrument, warn};

const ARTIFACT_MAX_BYTES: usize = 100 * 1024 * 1024;

pub struct RegistryStorage {
    store: Arc<dyn ObjectStore>,
    prefix: String,
    tenant_id: String,
    manifest_verifier_public_key: Vec<u8>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
/// Digest payload for `manifest_payload_digest`.
///
/// Hash stability depends on the serde serialization shape of this payload and
/// all nested types referenced here (`Route`, `Dependencies`, `Kind`).
/// Changing serde attributes (for example `rename_all`, field/variant renames,
/// or ordering-affecting schema changes) can silently change computed digests.
/// Treat such serde-shape changes as breaking: update stored manifests, add
/// migration steps, and add digest regression tests.
///
/// Nested JSON object fields are canonicalized in `normalize_value`, which
/// recursively sorts object keys lexicographically before serialization.
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
    pub fn new(
        store: Arc<dyn ObjectStore>,
        tenant_ctx: &TenantContext,
        manifest_verifier_public_key: Vec<u8>,
    ) -> Self {
        Self {
            store,
            prefix: format!("tenants/{}/", tenant_ctx.tenant_id().as_str()),
            tenant_id: tenant_ctx.tenant_id().to_string(),
            manifest_verifier_public_key,
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

    pub fn manifest_path(&self, id: &str, version: &str) -> Path {
        Path::from(format!(
            "{}manifests/{}/{}/manifest.json",
            self.prefix, id, version
        ))
    }

    /// Computes a digest from the JSON serialization of `ManifestDigestPayload`.
    ///
    /// Maintenance note: digest stability is tied to serde configuration of
    /// nested payload types (`Route`, `Dependencies`, `Kind`) and this payload's
    /// own serde shape. Any serde change (including `rename_all`, field/variant
    /// renames, or ordering-affecting schema edits) must be treated as breaking:
    /// update stored manifests, add migration steps, and add digest regression
    /// tests.
    ///
    /// Canonicalization contract: nested object values are normalized by
    /// `normalize_value`, including lexicographic key sorting for all
    /// `serde_json::Value::Object` nodes.
    fn manifest_payload_digest(manifest: &DistManifest) -> Result<String, RegistryError> {
        let payload_bytes = Self::manifest_payload_bytes(manifest)?;
        let mut hasher = Sha384::new();
        hasher.update(payload_bytes);
        Ok(hex::encode(hasher.finalize()))
    }

    fn manifest_payload_bytes(manifest: &DistManifest) -> Result<Vec<u8>, RegistryError> {
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

        // Normalize values to ensure digest stability (numeric canonicalization + sorted object keys).
        let payload_value = serde_json::to_value(&payload)?;
        let normalized = Self::normalize_value(payload_value);
        serde_json::to_vec(&normalized).map_err(RegistryError::from)
    }

    fn manifest_signature_payload(manifest: &DistManifest, digest: &str) -> String {
        format!(
            "flexisuite-registry-manifest:v1:{}:{}:{}:{}",
            manifest.id, manifest.version, manifest.security.manifest_signature_kid, digest
        )
    }

    pub fn signature_payload_for_signing(
        manifest: &DistManifest,
    ) -> Result<Vec<u8>, RegistryError> {
        if manifest.id.contains(':')
            || manifest.version.contains(':')
            || manifest.security.manifest_signature_kid.contains(':')
        {
            return Err(RegistryError::InvalidManifest(
                "id/version/manifest_signature_kid must not contain ':'".to_string(),
            ));
        }
        let digest = Self::manifest_payload_digest(manifest)?;
        Ok(Self::manifest_signature_payload(manifest, &digest).into_bytes())
    }

    fn verify_manifest_signature(
        &self,
        manifest: &DistManifest,
        computed_digest: &str,
    ) -> Result<(), RegistryError> {
        if self.manifest_verifier_public_key.len() != 32 {
            return Err(RegistryError::IntegrityCheckFailed {
                expected: "32-byte Ed25519 public key".to_string(),
                actual: format!(
                    "public key length {} bytes",
                    self.manifest_verifier_public_key.len()
                ),
            });
        }

        let signature_hex = manifest
            .security
            .manifest_signature
            .strip_prefix("ed25519:")
            .ok_or_else(|| {
                RegistryError::InvalidManifest(
                    "security.manifest_signature must start with 'ed25519:'".to_string(),
                )
            })?;
        let signature =
            hex::decode(signature_hex).map_err(|_| RegistryError::IntegrityCheckFailed {
                expected: "hex-encoded ed25519 signature".to_string(),
                actual: "invalid manifest signature encoding".to_string(),
            })?;

        let payload = Self::manifest_signature_payload(manifest, computed_digest);
        UnparsedPublicKey::new(&ED25519, &self.manifest_verifier_public_key)
            .verify(payload.as_bytes(), &signature)
            .map_err(|_| RegistryError::IntegrityCheckFailed {
                expected: "valid manifest signature".to_string(),
                actual: "manifest signature verification failed".to_string(),
            })
    }

    fn normalize_value(v: serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match v {
            Value::Object(map) => {
                let mut entries: Vec<(String, Value)> = map
                    .into_iter()
                    .map(|(k, v)| (k, Self::normalize_value(v)))
                    .collect();
                entries.sort_by(|(a, _), (b, _)| a.cmp(b));
                Value::Object(entries.into_iter().collect())
            }
            Value::Array(vec) => Value::Array(vec.into_iter().map(Self::normalize_value).collect()),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    return Value::from(i);
                }
                if let Some(u) = n.as_u64() {
                    return Value::from(u);
                }
                if let Some(f) = n.as_f64() {
                    // Check if the float is effectively an integer
                    if f.fract() == 0.0 {
                        // Prefer integer representation if it fits in i64/u64
                        // Perform lossless round-trip check to avoid silent saturation
                        if f >= (i64::MIN as f64) && f < (i64::MAX as f64) {
                            let i = f as i64;
                            if (i as f64) == f {
                                return Value::from(i);
                            }
                        }
                        if f >= 0.0 && f < (u64::MAX as f64) {
                            let u = f as u64;
                            if (u as f64) == f {
                                return Value::from(u);
                            }
                        }
                    }
                }
                Value::Number(n)
            }
            _ => v,
        }
    }

    /// Saves binary data and returns the SHA-384 digest (hex string).
    #[instrument(skip(self, data), fields(tenant = %self.tenant_id, artifact = %key))]
    pub async fn save_artifact(&self, key: &str, data: Bytes) -> Result<String, RegistryError> {
        Self::validate_key(key)?;
        let mut hasher = Sha384::new();
        hasher.update(&data);
        let digest = hex::encode(hasher.finalize());

        let path = self.artifact_path(key);
        if let Err(e) = self.store.put(&path, data.into()).await {
            error!("Failed to save artifact: {}", e);
            return Err(RegistryError::ObjectStore(e));
        }

        info!(digest = %digest, "Artifact saved successfully");
        Ok(digest)
    }

    /// Retrieves binary data. If expected_digest is provided, verifies SHA-384.
    #[instrument(skip(self), fields(tenant = %self.tenant_id, artifact = %key))]
    pub async fn get_artifact(
        &self,
        key: &str,
        expected_digest: Option<&str>,
    ) -> Result<Bytes, RegistryError> {
        Self::validate_key(key)?;
        let path = self.artifact_path(key);
        let result = self.store.get(&path).await.map_err(|e| match e {
            object_store::Error::NotFound { .. } => {
                warn!("Artifact not found");
                RegistryError::ArtifactNotFound(key.to_string())
            }
            _ => {
                error!("Object store error: {}", e);
                RegistryError::ObjectStore(e)
            }
        })?;
        let mut data = Vec::new();
        let mut stream = result.into_stream();
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result?;
            if data.len().saturating_add(chunk.len()) > ARTIFACT_MAX_BYTES {
                return Err(RegistryError::ArtifactTooLarge {
                    max: ARTIFACT_MAX_BYTES,
                    actual: data.len().saturating_add(chunk.len()),
                });
            }
            data.extend_from_slice(&chunk);
        }
        let data = Bytes::from(data);

        if let Some(expected) = expected_digest {
            let mut hasher = Sha384::new();
            hasher.update(&data);
            let actual = hex::encode(hasher.finalize());
            if actual != expected {
                warn!(expected = %expected, actual = %actual, "Integrity check failed");
                return Err(RegistryError::IntegrityCheckFailed {
                    expected: expected.to_string(),
                    actual,
                });
            }
        }

        info!("Artifact retrieved successfully");
        Ok(data)
    }

    /// Saves a DistManifest to `manifests/{id}/{version}/manifest.json`.
    /// Returns the SHA-384 digest and persisted manifest with manifest_digest set.
    #[instrument(skip(self, manifest), fields(tenant = %self.tenant_id, manifest.id = %manifest.id, manifest.version = %manifest.version))]
    pub async fn save_manifest(
        &self,
        manifest: &DistManifest,
    ) -> Result<(String, DistManifest), RegistryError> {
        Self::validate_key(&manifest.id)?;
        Self::validate_key(&manifest.version)?;
        if manifest.id.contains(':') || manifest.version.contains(':') {
            warn!("Manifest rejected: id/version contains colon");
            return Err(RegistryError::InvalidManifest(
                "id/version must not contain ':'".to_string(),
            ));
        }
        if manifest.security.manifest_signature.trim().is_empty() {
            warn!("Manifest rejected: empty signature");
            return Err(RegistryError::InvalidManifest(
                "security.manifest_signature must not be empty".to_string(),
            ));
        }
        if manifest.security.manifest_signature_kid.trim().is_empty() {
            warn!("Manifest rejected: empty signature kid");
            return Err(RegistryError::InvalidManifest(
                "security.manifest_signature_kid must not be empty".to_string(),
            ));
        }
        if manifest.security.manifest_signature_kid.contains(':') {
            warn!("Manifest rejected: signature kid contains colon");
            return Err(RegistryError::InvalidManifest(
                "security.manifest_signature_kid must not contain ':'".to_string(),
            ));
        }
        if manifest.security.trust_root_version.trim().is_empty() {
            warn!("Manifest rejected: empty trust root version");
            return Err(RegistryError::InvalidManifest(
                "security.trust_root_version must not be empty".to_string(),
            ));
        }

        // manifest_digest is computed from the manifest with the entire
        // security section excluded from the hashed payload.
        let computed_digest = Self::manifest_payload_digest(manifest)?;
        self.verify_manifest_signature(manifest, &computed_digest)?;
        let mut persisted = manifest.clone();
        persisted.security.manifest_digest = computed_digest.clone();

        let path = self.manifest_path(&manifest.id, &manifest.version);
        let data = serde_json::to_vec(&persisted)?;

        let put_result = self
            .store
            .put_opts(
                &path,
                data.into(),
                PutOptions {
                    mode: PutMode::Create,
                    ..Default::default()
                },
            )
            .await;
        if let Err(e) = put_result {
            return match e {
                object_store::Error::AlreadyExists { .. } => {
                    warn!("Manifest already exists and cannot be overwritten");
                    Err(RegistryError::ManifestAlreadyExists(format!(
                        "{}/{}",
                        manifest.id, manifest.version
                    )))
                }
                _ => {
                    error!("Failed to save manifest: {}", e);
                    Err(RegistryError::ObjectStore(e))
                }
            };
        }

        info!(digest = %computed_digest, "Manifest saved successfully");
        Ok((computed_digest, persisted))
    }

    /// Retrieves a DistManifest from `manifests/{id}/{version}/manifest.json`.
    #[instrument(skip(self), fields(tenant = %self.tenant_id, manifest.id = %id, manifest.version = %version))]
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
                warn!("Manifest not found");
                RegistryError::ManifestNotFound(format!("{id}/{version}"))
            }
            _ => {
                error!("Object store error: {}", e);
                RegistryError::ObjectStore(e)
            }
        })?;
        let data = result.bytes().await?;
        let manifest: DistManifest = serde_json::from_slice(&data)?;
        let actual = Self::manifest_payload_digest(&manifest)?;
        let expected = manifest.security.manifest_digest.clone();
        if actual != expected {
            warn!(expected = %expected, actual = %actual, "Manifest integrity check failed");
            return Err(RegistryError::IntegrityCheckFailed { expected, actual });
        }
        self.verify_manifest_signature(&manifest, &actual)?;
        info!(digest = %actual, "Manifest retrieved successfully");
        Ok(manifest)
    }
}
