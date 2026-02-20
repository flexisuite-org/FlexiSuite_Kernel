use crate::error::RegistryError;
use crate::model::{Dependencies, DistManifest, Kind, Route};
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use bytes::Bytes;
use ed25519_dalek::{Signature, VerifyingKey, Verifier};
use kernel_core::auth::TenantContext;
use object_store::ObjectStore;
use object_store::path::Path;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use sha2::{Digest, Sha384};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};
use tracing::{error, info, instrument, warn};

// Cached Trust Roots
static TRUST_ROOTS: Lazy<RwLock<HashMap<String, VerifyingKey>>> = Lazy::new(|| {
    let map = load_trust_roots_from_env();
    RwLock::new(map)
});

fn load_trust_roots_from_env() -> HashMap<String, VerifyingKey> {
    let mut map = HashMap::new();
    for (key, val) in std::env::vars() {
        if let Some(kid_suffix) = key.strip_prefix("FLEXI_REGISTRY_TRUST_ROOT_KEY_B64URL_") {
            if kid_suffix.is_empty() {
                continue;
            }
            // The suffix is the NORMALIZED KID (uppercase, alphanumeric + underscore).
            // We store it as-is for lookup.
            match BASE64_URL_SAFE_NO_PAD.decode(&val) {
                Ok(bytes) => {
                    if let Ok(vk) = VerifyingKey::from_bytes(bytes.as_slice().try_into().unwrap_or(&[0u8; 32])) {
                         map.insert(kid_suffix.to_string(), vk);
                    } else {
                        warn!("Invalid Ed25519 public key for env var {}", key);
                    }
                }
                Err(_) => {
                    warn!("Invalid Base64URL encoding for env var {}", key);
                }
            }
        }
    }
    info!("Loaded {} trust root keys from environment", map.len());
    map
}

/// Explicitly reloads trust root keys from environment variables.
/// Call this after updating environment variables (e.g., in tests or during config refresh).
pub fn reload_trust_root_keys() {
    let new_map = load_trust_roots_from_env();
    let mut write_guard = TRUST_ROOTS.write().expect("Trust root lock poisoned");
    *write_guard = new_map;
}

pub struct RegistryStorage {
    store: Arc<dyn ObjectStore>,
    prefix: String,
    tenant_id: String,
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
                    if f.fract() == 0.0 {
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

    fn verify_signature(kid: &str, digest: &str, signature: &str) -> Result<(), RegistryError> {
        // 1. Validate KID Format
        // Strict allowlist: Alphanumeric, hyphen, underscore only.
        static KID_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Za-z0-9_-]+$").unwrap());
        if !KID_REGEX.is_match(kid) {
            warn!(kid = %kid, "Invalid KID format rejected");
            return Err(RegistryError::KeyNotFound(format!("Invalid KID format: {}", kid)));
        }

        // 2. Normalize KID for Lookup
        // Convert to uppercase, replacing non-alphanumerics with underscore is NOT needed
        // because we already validated strict charset [A-Za-z0-9_-].
        // However, the ENV VAR convention (FLEXI_REGISTRY_TRUST_ROOT_KEY_B64URL_{NORMALIZED})
        // typically implies uppercase transformation for shell variable compatibility.
        // We replicate the normalization logic: Alphanumeric -> Uppercase, others -> Underscore.
        let normalized_kid = kid
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();

        // 3. Lookup Public Key from Cache
        let trust_roots = TRUST_ROOTS.read().expect("Trust root lock poisoned");
        let public_key = trust_roots.get(&normalized_kid).ok_or_else(|| {
            warn!(kid = %kid, normalized = %normalized_kid, "Trust root key not found");
            RegistryError::KeyNotFound(format!("Public key for kid '{kid}' not found"))
        })?;

        // 4. Decode Signature
        let signature_bytes = BASE64_URL_SAFE_NO_PAD.decode(signature).map_err(|e| {
            warn!(kid = %kid, "Invalid base64 signature");
            RegistryError::SignatureVerificationFailed(format!("Invalid signature encoding: {e}"))
        })?;
        let signature_obj = Signature::from_slice(&signature_bytes).map_err(|e| {
            warn!(kid = %kid, "Invalid signature length");
            RegistryError::SignatureVerificationFailed(format!("Invalid signature format: {e}"))
        })?;

        // 5. Verify (Ed25519)
        // Verify raw SHA-384 digest bytes (48 bytes), NOT the hex string bytes.
        let digest_bytes = hex::decode(digest).map_err(|e| {
            warn!(kid = %kid, "Invalid digest hex encoding");
            RegistryError::InvalidManifest(format!("Invalid digest hex: {e}"))
        })?;

        public_key.verify(&digest_bytes, &signature_obj).map_err(|_| {
            warn!(kid = %kid, digest = %digest, "Signature verification failed");
            RegistryError::SignatureVerificationFailed("Invalid signature".to_string())
        })
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
        if manifest.security.trust_root_version.trim().is_empty() {
            warn!("Manifest rejected: empty trust root version");
            return Err(RegistryError::InvalidManifest(
                "security.trust_root_version must not be empty".to_string(),
            ));
        }

        // manifest_digest is computed from the manifest with the entire
        // security section excluded from the hashed payload.
        let computed_digest = Self::manifest_payload_digest(manifest)?;

        // Verify signature BEFORE accepting/persisting
        Self::verify_signature(
            &manifest.security.manifest_signature_kid,
            &computed_digest,
            &manifest.security.manifest_signature,
        )?;

        let mut persisted = manifest.clone();
        persisted.security.manifest_digest = computed_digest.clone();

        let path = self.manifest_path(&manifest.id, &manifest.version);
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
        let manifest: DistManifest = serde_json::from_slice(&data)?;
        let actual = Self::manifest_payload_digest(&manifest)?;
        let expected = manifest.security.manifest_digest.clone();
        if actual != expected {
            warn!(expected = %expected, actual = %actual, "Manifest integrity check failed");
            return Err(RegistryError::IntegrityCheckFailed { expected, actual });
        }
        info!(digest = %actual, "Manifest retrieved successfully");
        Ok(manifest)
    }
}
