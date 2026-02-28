use crate::error::RegistryError;
use crate::model::{Dependencies, DistManifest, Kind, Route};
use crate::trust::TrustProvider;
use bytes::Bytes;
use kernel_core::auth::TenantContext;
use kernel_core::supplychain::{Manifest as CoreManifest, VerificationResult, verify_manifest};
use object_store::ObjectStore;
use object_store::path::Path;
use serde::Serialize;
use sha2::{Digest, Sha384};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{error, info, instrument, warn};

pub struct RegistryStorage {
    store: Arc<dyn ObjectStore>,
    trust_provider: Arc<dyn TrustProvider>,
    prefix: String,
    tenant_id: String,
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
        trust_provider: Arc<dyn TrustProvider>,
        tenant_ctx: &TenantContext,
    ) -> Self {
        Self {
            store,
            trust_provider,
            prefix: format!("tenants/{}/", tenant_ctx.tenant_id().as_str()),
            tenant_id: tenant_ctx.tenant_id().to_string(),
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

    /// Computes a digest from the JSON serialization of `ManifestDigestPayload`.
    ///
    /// Maintenance note: digest stability is tied to serde configuration of
    /// nested payload types (`Route`, `Dependencies`, `Kind`) and this payload's
    /// own serde shape. Any serde change (including `rename_all`, field/variant
    /// renames, or ordering-affecting schema edits) must be treated as breaking:
    /// update stored manifests, add migration steps, and add digest regression
    /// tests.
    pub fn manifest_payload_digest(manifest: &DistManifest) -> Result<String, RegistryError> {
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

        // Normalize numeric values to ensure digest stability (1.0 vs 1)
        let payload_value = serde_json::to_value(&payload)?;
        let normalized = Self::normalize_value(payload_value);
        let payload_bytes = serde_json::to_vec(&normalized)?;

        let mut hasher = Sha384::new();
        hasher.update(payload_bytes);
        Ok(hex::encode(hasher.finalize()))
    }

    fn normalize_value(v: serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match v {
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(k, v)| (k, Self::normalize_value(v)))
                    .collect(),
            ),
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

        let data = result.bytes().await?;

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

        let mut persisted = manifest.clone();

        // 1. Compute and Set/Normalize Digest
        // Computed digest from payload is always raw hex here.
        let computed_digest_hex = Self::manifest_payload_digest(&persisted)?;
        let computed_digest = format!("sha384-{}", computed_digest_hex);

        // If the incoming manifest already has a digest, it must match.
        // If it's empty, we fill it.
        if !persisted.security.manifest_digest.trim().is_empty() {
            let normalized_incoming = if persisted.security.manifest_digest.starts_with("sha384-") {
                persisted.security.manifest_digest.clone()
            } else {
                format!("sha384-{}", persisted.security.manifest_digest)
            };
            if normalized_incoming != computed_digest {
                warn!(expected = %computed_digest, actual = %normalized_incoming, "Manifest rejected: digest mismatch in save_manifest");
                return Err(RegistryError::IntegrityCheckFailed {
                    expected: computed_digest,
                    actual: normalized_incoming,
                });
            }
        }
        persisted.security.manifest_digest = computed_digest.clone();

        // 2. Comprehensive Validation & Signature Verification (Fail-Closed)
        self.verify_and_canonicalize_manifest(&mut persisted)?;

        let path = self.manifest_path(&persisted.id, &persisted.version);
        let data = serde_json::to_vec(&persisted)?;

        if let Err(e) = self.store.put(&path, data.into()).await {
            error!("Failed to save manifest: {}", e);
            return Err(RegistryError::ObjectStore(e));
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
        let mut manifest: DistManifest = serde_json::from_slice(&data)?;

        // Enforce Authenticity Check (Fail-Closed)
        self.verify_and_canonicalize_manifest(&mut manifest)?;

        info!(digest = %manifest.security.manifest_digest, "Manifest retrieved successfully");
        Ok(manifest)
    }

    /// Verifies manifest integrity and signature.
    /// Canonicalizes the manifest_digest to include the `sha384-` prefix.
    fn verify_and_canonicalize_manifest(
        &self,
        manifest: &mut DistManifest,
    ) -> Result<(), RegistryError> {
        // 1. Basic field presence
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
        let expected_version = self.trust_provider.trust_root_version();
        if manifest.security.trust_root_version != expected_version {
            warn!(
                expected = %expected_version,
                actual = %manifest.security.trust_root_version,
                "Manifest rejected: trust_root_version mismatch"
            );
            return Err(RegistryError::InvalidManifest(format!(
                "trust_root_version mismatch: expected {}, got {}",
                expected_version, manifest.security.trust_root_version
            )));
        }

        // 2. Digest Integrity
        let computed_digest_hex = Self::manifest_payload_digest(manifest)?;
        let computed_digest = format!("sha384-{}", computed_digest_hex);

        let stored_digest = &manifest.security.manifest_digest;
        if stored_digest.trim().is_empty() {
            return Err(RegistryError::InvalidManifest(
                "security.manifest_digest must not be empty".to_string(),
            ));
        }

        let normalized_stored = if stored_digest.starts_with("sha384-") {
            stored_digest.clone()
        } else {
            format!("sha384-{}", stored_digest)
        };

        if computed_digest != normalized_stored {
            warn!(expected = %normalized_stored, actual = %computed_digest, "Manifest integrity check failed");
            return Err(RegistryError::IntegrityCheckFailed {
                expected: normalized_stored,
                actual: computed_digest,
            });
        }

        // 3. Signature Verification (Authenticity)
        let trusted_key = self
            .trust_provider
            .get_key(&manifest.security.manifest_signature_kid)?;

        let core_manifest = CoreManifest {
            id: manifest.id.clone(),
            digest: normalized_stored.clone(),
            signature: manifest.security.manifest_signature.clone(),
            kid: manifest.security.manifest_signature_kid.clone(),
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| {
                RegistryError::InvalidManifest(format!("System time before UNIX EPOCH: {}", e))
            })?
            .as_secs();

        match verify_manifest(&core_manifest, &trusted_key, &normalized_stored, now) {
            VerificationResult::Ok => {
                // Canonicalize
                manifest.security.manifest_digest = normalized_stored;
                Ok(())
            }
            res => {
                warn!(result = ?res, manifest.id = %manifest.id, "Manifest signature verification failed");
                Err(RegistryError::InvalidManifest(format!(
                    "Signature verification failed: {:?}",
                    res
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::tests::MockTrustProvider;
    use kernel_core::auth::{TenantContext, TenantId};
    use kernel_core::supplychain::{KeyStatus, TrustedKey};
    use object_store::memory::InMemory;
    use object_store::path::Path;
    use std::collections::BTreeMap;

    fn test_tenant_ctx() -> TenantContext {
        TenantContext::new(TenantId::new("tenant_test").expect("valid tenant id"), None)
    }

    fn mock_trust_provider() -> Arc<dyn TrustProvider> {
        let mut provider = MockTrustProvider::new();
        provider.add_key(TrustedKey {
            kid: "key1".to_string(),
            alg: "Ed25519".to_string(),
            status: KeyStatus::Active,
            retired_at: None,
            not_before: None,
            not_after: None,
            public_key: [0u8; 32],
        });
        provider.add_key(TrustedKey {
            kid: "kid_default".to_string(),
            alg: "Ed25519".to_string(),
            status: KeyStatus::Active,
            retired_at: None,
            not_before: None,
            not_after: None,
            public_key: [0u8; 32],
        });
        provider.add_key(TrustedKey {
            kid: "kid_A".to_string(),
            alg: "Ed25519".to_string(),
            status: KeyStatus::Active,
            retired_at: None,
            not_before: None,
            not_after: None,
            public_key: [0u8; 32],
        });
        provider.add_key(TrustedKey {
            kid: "kid_B".to_string(),
            alg: "Ed25519".to_string(),
            status: KeyStatus::Active,
            retired_at: None,
            not_before: None,
            not_after: None,
            public_key: [0u8; 32],
        });
        provider.add_key(TrustedKey {
            kid: "kid".to_string(),
            alg: "Ed25519".to_string(),
            status: KeyStatus::Active,
            retired_at: None,
            not_before: None,
            not_after: None,
            public_key: [0u8; 32],
        });
        Arc::new(provider)
    }

    fn test_manifest(id: &str, version: &str) -> DistManifest {
        DistManifest {
            schema_version: "1.0".to_string(),
            id: id.to_string(),
            version: version.into(),
            kind: Kind::Composition,
            name: "Test App".to_string(),
            protected: false,
            composition_root: "main.tsx".to_string(),
            routes: vec![Route {
                path: "/".to_string(),
                component: "layout".to_string(),
            }],
            dependencies: Dependencies {
                components: BTreeMap::new(),
                permissions: vec![],
            },
            configuration: BTreeMap::new(),
            security: crate::model::Security {
                manifest_digest: "".to_string(),
                manifest_signature: "sig_default".to_string(),
                manifest_signature_kid: "kid_default".to_string(),
                trust_root_version: "v1".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn test_save_and_get_artifact() {
        let store = Arc::new(InMemory::new());
        let registry = RegistryStorage::new(store, mock_trust_provider(), &test_tenant_ctx());

        let data = b"hello world";
        let digest = registry
            .save_artifact("test/file.txt", data.as_slice().into())
            .await
            .unwrap();

        assert_eq!(
            digest,
            "fdbd8e75a67f29f701a4e040385e2e23986303ea10239211af907fcbb83578b3e417cb71ce646efd0819dd8c088de1bd"
        );

        let retrieved = registry
            .get_artifact("test/file.txt", Some(&digest))
            .await
            .unwrap();
        assert_eq!(retrieved, data.as_slice());

        let result = registry
            .get_artifact("test/file.txt", Some("bad_digest"))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_save_and_get_manifest() {
        let store = Arc::new(InMemory::new());
        let registry = RegistryStorage::new(store, mock_trust_provider(), &test_tenant_ctx());

        let mut manifest = test_manifest("app_test", "1.0.0");
        manifest.security.manifest_signature = "sig_...".to_string();
        manifest.security.manifest_signature_kid = "key1".to_string();

        let (digest, persisted) = registry.save_manifest(&manifest).await.unwrap();
        assert_eq!(digest.len(), 103);
        assert_eq!(persisted.security.manifest_digest, digest);

        let retrieved = registry.get_manifest("app_test", "1.0.0").await.unwrap();
        assert_eq!(retrieved.security.manifest_digest, digest);
        assert_eq!(retrieved.schema_version, manifest.schema_version);
        assert_eq!(retrieved.id, manifest.id);
        assert_eq!(retrieved.version, manifest.version);
        assert_eq!(retrieved.kind, manifest.kind);
        assert_eq!(retrieved.name, manifest.name);
        assert_eq!(retrieved.protected, manifest.protected);
        assert_eq!(retrieved.composition_root, manifest.composition_root);
        assert_eq!(retrieved.routes, manifest.routes);
        assert_eq!(retrieved.dependencies, manifest.dependencies);
        assert_eq!(retrieved.configuration, manifest.configuration);
        assert_eq!(
            retrieved.security.manifest_signature,
            manifest.security.manifest_signature
        );
        assert_eq!(
            retrieved.security.manifest_signature_kid,
            manifest.security.manifest_signature_kid
        );
        assert_eq!(
            retrieved.security.trust_root_version,
            manifest.security.trust_root_version
        );
    }

    #[tokio::test]
    async fn test_manifest_digest_excludes_security_section() {
        let store_a = Arc::new(InMemory::new());
        let registry_a = RegistryStorage::new(store_a, mock_trust_provider(), &test_tenant_ctx());

        let store_b = Arc::new(InMemory::new());
        let registry_b = RegistryStorage::new(store_b, mock_trust_provider(), &test_tenant_ctx());

        let mut manifest_a = test_manifest("app_security_digest", "1.0.0");
        manifest_a.name = "Security Digest Test".to_string();
        manifest_a.security.manifest_signature = "sig_A".to_string();
        manifest_a.security.manifest_signature_kid = "kid_A".to_string();

        let mut manifest_b = manifest_a.clone();
        manifest_b.security.manifest_signature = "sig_B".to_string();
        manifest_b.security.manifest_signature_kid = "kid_B".to_string();
        manifest_b.security.trust_root_version = "v1".to_string();

        let (digest_a, _) = registry_a.save_manifest(&manifest_a).await.unwrap();
        let (digest_b, _) = registry_b.save_manifest(&manifest_b).await.unwrap();

        assert_eq!(digest_a, digest_b);
    }

    #[tokio::test]
    async fn test_get_missing_manifest_returns_manifest_not_found() {
        let store = Arc::new(InMemory::new());
        let registry = RegistryStorage::new(store, mock_trust_provider(), &test_tenant_ctx());

        let result = registry.get_manifest("missing_id", "0.0.0").await;
        match result {
            Err(RegistryError::ManifestNotFound(path)) => {
                assert_eq!(path, "missing_id/0.0.0");
            }
            other => panic!("expected ManifestNotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_save_manifest_rejects_empty_security_fields() {
        let store = Arc::new(InMemory::new());
        let registry = RegistryStorage::new(store, mock_trust_provider(), &test_tenant_ctx());

        let mut manifest = test_manifest("app_invalid_security", "1.0.0");
        manifest.name = "Invalid Security".to_string();
        manifest.security.manifest_signature = "".to_string();
        manifest.security.manifest_signature_kid = "key1".to_string();

        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::InvalidManifest(msg)) => {
                assert_eq!(msg, "security.manifest_signature must not be empty");
            }
            other => panic!("expected InvalidManifest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_save_manifest_rejects_empty_security_kid() {
        let store = Arc::new(InMemory::new());
        let registry = RegistryStorage::new(store, mock_trust_provider(), &test_tenant_ctx());

        let mut manifest = test_manifest("app_invalid_security_kid", "1.0.0");
        manifest.name = "Invalid Security Kid".to_string();
        manifest.security.manifest_signature = "sig_...".to_string();
        manifest.security.manifest_signature_kid = "".to_string();

        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::InvalidManifest(msg)) => {
                assert_eq!(msg, "security.manifest_signature_kid must not be empty");
            }
            other => panic!("expected InvalidManifest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_save_manifest_rejects_empty_trust_root_version() {
        let store = Arc::new(InMemory::new());
        let registry = RegistryStorage::new(store, mock_trust_provider(), &test_tenant_ctx());

        let mut manifest = test_manifest("app_invalid_trust_root_version", "1.0.0");
        manifest.name = "Invalid Trust Root Version".to_string();
        manifest.security.manifest_signature = "sig_...".to_string();
        manifest.security.manifest_signature_kid = "key1".to_string();
        manifest.security.trust_root_version = "".to_string();

        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::InvalidManifest(msg)) => {
                assert_eq!(msg, "security.trust_root_version must not be empty");
            }
            other => panic!("expected InvalidManifest, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_get_manifest_detects_tampered_stored_json() {
        let store = Arc::new(InMemory::new());
        let tenant_ctx = test_tenant_ctx();
        let registry = RegistryStorage::new(store.clone(), mock_trust_provider(), &tenant_ctx);

        let mut manifest = test_manifest("app_tamper_test", "1.0.0");
        manifest.name = "Tamper Test".to_string();
        manifest.security.manifest_signature = "sig_...".to_string();
        manifest.security.manifest_signature_kid = "key1".to_string();

        let (_, persisted) = registry.save_manifest(&manifest).await.unwrap();

        let mut tampered = persisted.clone();
        tampered.name = "Tampered Name".to_string();
        let tampered_bytes = serde_json::to_vec(&tampered).unwrap();
        let tampered_path = Path::from(format!(
            "tenants/{}/manifests/{}/{}/manifest.json",
            tenant_ctx.tenant_id().as_str(),
            manifest.id,
            manifest.version
        ));
        store
            .put(&tampered_path, tampered_bytes.into())
            .await
            .unwrap();

        let result = registry.get_manifest(&manifest.id, &manifest.version).await;
        match result {
            Err(RegistryError::IntegrityCheckFailed { .. }) => {}
            other => panic!("expected IntegrityCheckFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_manifest_digest_auto_canonicalization() {
        let store = Arc::new(InMemory::new());
        let registry = RegistryStorage::new(store, mock_trust_provider(), &test_tenant_ctx());

        let mut manifest = test_manifest("app_canonical", "1.0.0");
        manifest.security.manifest_signature = "sig".to_string();
        manifest.security.manifest_signature_kid = "key1".to_string();
        let computed_hex = RegistryStorage::manifest_payload_digest(&manifest).unwrap();
        manifest.security.manifest_digest = computed_hex;

        let (digest, persisted) = registry.save_manifest(&manifest).await.unwrap();
        assert!(digest.starts_with("sha384-"));
        assert!(persisted.security.manifest_digest.starts_with("sha384-"));

        let retrieved = registry
            .get_manifest("app_canonical", "1.0.0")
            .await
            .unwrap();
        assert!(retrieved.security.manifest_digest.starts_with("sha384-"));
    }

    #[tokio::test]
    async fn test_manifest_verification_fail_closed() {
        let store = Arc::new(InMemory::new());
        let provider = MockTrustProvider::new();
        let registry = RegistryStorage::new(store, Arc::new(provider), &test_tenant_ctx());

        let mut manifest = test_manifest("app_fail_closed", "1.0.0");
        manifest.security.manifest_signature = "sig".to_string();
        manifest.security.manifest_signature_kid = "key1".to_string();

        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::TrustRootError(msg)) => {
                assert!(msg.contains("Key not found: key1"));
            }
            other => panic!("expected TrustRootError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_manifest_invalid_signature_fail_closed() {
        let store = Arc::new(InMemory::new());
        let mut provider = MockTrustProvider::new();
        provider.add_key(TrustedKey {
            kid: "key1".to_string(),
            alg: "Ed25519".to_string(),
            status: KeyStatus::Active,
            retired_at: None,
            not_before: None,
            not_after: None,
            public_key: [0u8; 32],
        });
        let registry = RegistryStorage::new(store, Arc::new(provider), &test_tenant_ctx());

        let mut manifest = test_manifest("app_bad_sig", "1.0.0");
        manifest.security.manifest_signature = "invalid".to_string();
        manifest.security.manifest_signature_kid = "key1".to_string();

        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::InvalidManifest(msg)) => {
                assert!(msg.contains("Signature verification failed: SignatureInvalid"));
            }
            other => panic!("expected InvalidManifest(SignatureInvalid), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_save_manifest_rejects_control_character_in_id() {
        let store = Arc::new(InMemory::new());
        let registry = RegistryStorage::new(store, mock_trust_provider(), &test_tenant_ctx());

        let mut manifest = test_manifest("app_\0_test", "1.0.0");
        manifest.name = "Invalid Key".to_string();
        manifest.security.manifest_signature = "sig".to_string();
        manifest.security.manifest_signature_kid = "kid".to_string();

        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::InvalidPath(msg)) => {
                assert_eq!(
                    msg,
                    format!("invalid key contains control character: {}", manifest.id)
                );
            }
            other => panic!("expected InvalidPath, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_manifest_digest_numeric_normalization() {
        let store_a = Arc::new(InMemory::new());
        let registry_a = RegistryStorage::new(store_a, mock_trust_provider(), &test_tenant_ctx());

        let store_b = Arc::new(InMemory::new());
        let registry_b = RegistryStorage::new(store_b, mock_trust_provider(), &test_tenant_ctx());

        let mut manifest_int = test_manifest("app_numeric", "1.0.0");
        manifest_int
            .configuration
            .insert("count".to_string(), serde_json::json!(1));

        let mut manifest_float = test_manifest("app_numeric", "1.0.0");
        manifest_float
            .configuration
            .insert("count".to_string(), serde_json::json!(1.0));

        let (digest_int, _) = registry_a.save_manifest(&manifest_int).await.unwrap();
        let (digest_float, _) = registry_b.save_manifest(&manifest_float).await.unwrap();

        assert_eq!(
            digest_int, digest_float,
            "Digests should match for 1 and 1.0"
        );
    }

    #[tokio::test]
    async fn test_manifest_digest_big_int_normalization() {
        let store_a = Arc::new(InMemory::new());
        let registry_a = RegistryStorage::new(store_a, mock_trust_provider(), &test_tenant_ctx());

        let mut manifest_big = test_manifest("app_big_int", "1.0.0");
        manifest_big
            .configuration
            .insert("big_i64".to_string(), serde_json::json!(i64::MAX));
        manifest_big
            .configuration
            .insert("big_u64".to_string(), serde_json::json!(u64::MAX));

        let (digest, _) = registry_a.save_manifest(&manifest_big).await.unwrap();
        assert_eq!(digest.len(), 103);
    }

    #[tokio::test]
    async fn test_get_artifact_returns_artifact_not_found_for_missing_key() {
        let store = Arc::new(InMemory::new());
        let registry = RegistryStorage::new(store, mock_trust_provider(), &test_tenant_ctx());

        let result = registry.get_artifact("missing_artifact", None).await;
        match result {
            Err(RegistryError::ArtifactNotFound(key)) => {
                assert_eq!(key, "missing_artifact");
            }
            other => panic!("expected ArtifactNotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_registry_key_validation_invalid_paths() {
        let store = Arc::new(InMemory::new());
        let registry = RegistryStorage::new(store, mock_trust_provider(), &test_tenant_ctx());

        let invalid_keys = vec![
            "../traversal",
            "key\\backslash",
            "encode%2f",
            "encode%5c",
            "",
            "..",
            ".",
            "/leading",
            "a//b",
        ];

        for key in invalid_keys {
            let result = registry.save_artifact(key, b"data".as_slice().into()).await;
            match result {
                Err(RegistryError::InvalidPath(_)) => {}
                other => panic!("save_artifact: expected InvalidPath for {key}, got {other:?}"),
            }

            let manifest = test_manifest(key, "1.0.0");
            let result = registry.save_manifest(&manifest).await;
            match result {
                Err(RegistryError::InvalidPath(_)) => {}
                other => {
                    panic!("save_manifest: expected InvalidPath for {key} (id), got {other:?}")
                }
            }
        }
    }

    #[tokio::test]
    async fn test_save_manifest_rejects_whitespace_security_fields() {
        let store = Arc::new(InMemory::new());
        let registry = RegistryStorage::new(store, mock_trust_provider(), &test_tenant_ctx());

        let mut manifest = test_manifest("app_whitespace_sig", "1.0.0");
        manifest.security.manifest_signature = "   ".to_string();
        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::InvalidManifest(msg)) => {
                assert_eq!(msg, "security.manifest_signature must not be empty");
            }
            other => panic!("expected InvalidManifest for whitespace signature, got {other:?}"),
        }

        let mut manifest = test_manifest("app_whitespace_kid", "1.0.0");
        manifest.security.manifest_signature_kid = "   ".to_string();
        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::InvalidManifest(msg)) => {
                assert_eq!(msg, "security.manifest_signature_kid must not be empty");
            }
            other => panic!("expected InvalidManifest for whitespace kid, got {other:?}"),
        }

        let mut manifest = test_manifest("app_whitespace_trust", "1.0.0");
        manifest.security.trust_root_version = "   ".to_string();
        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::InvalidManifest(msg)) => {
                assert_eq!(msg, "security.trust_root_version must not be empty");
            }
            other => {
                panic!("expected InvalidManifest for whitespace trust_root_version, got {other:?}")
            }
        }
    }
}
