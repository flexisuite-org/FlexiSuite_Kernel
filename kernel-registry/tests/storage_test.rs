// ... imports ...
use kernel_core::auth::{TenantContext, TenantId};
use kernel_core::supplychain::{KeyStatus, TrustedKey};
use kernel_registry::error::RegistryError;
use kernel_registry::model::{Dependencies, DistManifest, Kind, Route, Security};
use kernel_registry::storage::RegistryStorage;
use kernel_registry::trust::TrustProvider;
use object_store::ObjectStore;
use object_store::memory::InMemory;
use object_store::path::Path;
use std::collections::BTreeMap;
use std::sync::Arc;
use ed25519_dalek::{SigningKey, Signer, VerifyingKey};
use rand::rngs::OsRng;
use std::collections::HashMap;
use sha2::Digest;

struct MockTrustProvider {
    keys: HashMap<String, TrustedKey>,
}

impl MockTrustProvider {
    fn new() -> Self {
        Self { keys: HashMap::new() }
    }
    fn add(&mut self, key: TrustedKey) {
        self.keys.insert(key.kid.clone(), key);
    }
}

impl TrustProvider for MockTrustProvider {
    fn get_key(&self, kid: &str) -> Result<TrustedKey, RegistryError> {
        self.keys.get(kid).cloned().ok_or_else(|| {
            RegistryError::TrustRootError(format!("Key not found: {}", kid))
        })
    }
}

fn test_tenant_ctx() -> TenantContext {
    TenantContext::new(TenantId::new("tenant_test").expect("valid tenant id"), None)
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
        security: Security {
            manifest_digest: "".to_string(),
            manifest_signature: "".to_string(),
            manifest_signature_kid: "".to_string(),
            trust_root_version: "v1".to_string(),
        },
    }
}

fn setup_registry_with_keys(store: Arc<dyn ObjectStore>) -> (RegistryStorage, SigningKey, String) {
    let mut csprng = OsRng;
    let signing_key = SigningKey::generate(&mut csprng);
    let verifying_key = signing_key.verifying_key();
    let pub_key_hex = hex::encode(verifying_key.to_bytes());
    let kid = "test_key".to_string();

    let trusted_key = TrustedKey {
        kid: kid.clone(),
        alg: "Ed25519".to_string(),
        public_key: pub_key_hex,
        status: KeyStatus::Active,
        retired_at: None,
        not_before: None,
        not_after: None,
    };

    let mut mock_trust = MockTrustProvider::new();
    mock_trust.add(trusted_key);

    let registry = RegistryStorage::new(store, &test_tenant_ctx())
        .expect("Failed to create registry")
        .with_trust_provider(Arc::new(mock_trust));

    (registry, signing_key, kid)
}

#[tokio::test]
async fn test_save_and_get_artifact() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx()).unwrap();
    // Artifact saving doesn't require signature verification

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
}

fn compute_digest(manifest: &DistManifest) -> String {
    // Replicate logic
    #[derive(serde::Serialize)]
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

    fn normalize_value(v: serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match v {
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(k, v)| (k, normalize_value(v)))
                    .collect(),
            ),
            Value::Array(vec) => Value::Array(vec.into_iter().map(normalize_value).collect()),
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

    let payload_value = serde_json::to_value(&payload).unwrap();
    let normalized = normalize_value(payload_value);
    let payload_bytes = serde_json::to_vec(&normalized).unwrap();

    let mut hasher = sha2::Sha384::new();
    sha2::Digest::update(&mut hasher, payload_bytes);
    // Backwards compatibility: return raw hex, no prefix
    hex::encode(hasher.finalize())
}

fn compute_digest_prefixed(manifest: &DistManifest) -> String {
    format!("sha384-{}", compute_digest(manifest))
}

#[tokio::test]
async fn test_save_and_get_manifest() {
    let store = Arc::new(InMemory::new());
    let (registry, signing_key, kid) = setup_registry_with_keys(store);

    let mut manifest = test_manifest("app_test", "1.0.0");
    manifest.security.manifest_signature_kid = kid;

    // Sign the PREFIXED digest (this is required by verify_manifest contract)
    let digest_prefixed = compute_digest_prefixed(&manifest);
    let signature = hex::encode(signing_key.sign(digest_prefixed.as_bytes()).to_bytes());
    manifest.security.manifest_signature = signature;

    let (saved_digest, persisted) = registry.save_manifest(&manifest).await.unwrap();
    // Saved digest should be raw hex (no prefix)
    assert_eq!(saved_digest, compute_digest(&manifest));
    assert_eq!(persisted.security.manifest_digest, compute_digest(&manifest));

    let retrieved = registry.get_manifest("app_test", "1.0.0").await.unwrap();
    assert_eq!(retrieved.security.manifest_digest, compute_digest(&manifest));
}

#[tokio::test]
async fn test_manifest_digest_excludes_security_section() {
    let store_a = Arc::new(InMemory::new());
    let (registry_a, key_a, kid_a) = setup_registry_with_keys(store_a);

    let store_b = Arc::new(InMemory::new());
    let (registry_b, key_b, kid_b) = setup_registry_with_keys(store_b);

    let mut manifest_a = test_manifest("app_security_digest", "1.0.0");
    manifest_a.name = "Security Digest Test".to_string();
    manifest_a.security.manifest_signature_kid = kid_a;
    let digest_a_prefixed = compute_digest_prefixed(&manifest_a);
    manifest_a.security.manifest_signature = hex::encode(key_a.sign(digest_a_prefixed.as_bytes()).to_bytes());

    let mut manifest_b = manifest_a.clone();
    // Intentionally keep `manifest_b` with the same id/version/payload as `manifest_a`.
    manifest_b.security.manifest_signature_kid = kid_b;
    manifest_b.security.trust_root_version = "v2".to_string();
    let digest_b_prefixed = compute_digest_prefixed(&manifest_b);
    // Digest should be same as A
    assert_eq!(digest_a_prefixed, digest_b_prefixed);

    manifest_b.security.manifest_signature = hex::encode(key_b.sign(digest_b_prefixed.as_bytes()).to_bytes());

    let (digest_a, _) = registry_a.save_manifest(&manifest_a).await.unwrap();
    let (digest_b, _) = registry_b.save_manifest(&manifest_b).await.unwrap();

    assert_eq!(digest_a, digest_b);
}

#[tokio::test]
async fn test_save_manifest_rejects_empty_security_fields() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx()).unwrap();

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
    let registry = RegistryStorage::new(store, &test_tenant_ctx()).unwrap();

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
    let registry = RegistryStorage::new(store, &test_tenant_ctx()).unwrap();

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
    let (registry, key, kid) = setup_registry_with_keys(store.clone());
    let tenant_ctx = test_tenant_ctx();

    let mut manifest = test_manifest("app_tamper_test", "1.0.0");
    manifest.name = "Tamper Test".to_string();
    manifest.security.manifest_signature_kid = kid;
    let digest_prefixed = compute_digest_prefixed(&manifest);
    manifest.security.manifest_signature = hex::encode(key.sign(digest_prefixed.as_bytes()).to_bytes());

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
async fn test_save_manifest_rejects_control_character_in_id() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx()).unwrap();

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
    let (registry_a, key_a, kid_a) = setup_registry_with_keys(store_a);

    let store_b = Arc::new(InMemory::new());
    let (registry_b, key_b, kid_b) = setup_registry_with_keys(store_b);

    let mut manifest_int = test_manifest("app_numeric", "1.0.0");
    manifest_int
        .configuration
        .insert("count".to_string(), serde_json::json!(1));
    manifest_int.security.manifest_signature_kid = kid_a;
    let digest_int_prefixed = compute_digest_prefixed(&manifest_int);
    manifest_int.security.manifest_signature = hex::encode(key_a.sign(digest_int_prefixed.as_bytes()).to_bytes());

    let mut manifest_float = test_manifest("app_numeric", "1.0.0");
    manifest_float
        .configuration
        .insert("count".to_string(), serde_json::json!(1.0));
    manifest_float.security.manifest_signature_kid = kid_b;
    let digest_float_prefixed = compute_digest_prefixed(&manifest_float);
    manifest_float.security.manifest_signature = hex::encode(key_b.sign(digest_float_prefixed.as_bytes()).to_bytes());

    let (digest_int_saved, _) = registry_a.save_manifest(&manifest_int).await.unwrap();
    let (digest_float_saved, _) = registry_b.save_manifest(&manifest_float).await.unwrap();

    assert_eq!(
        digest_int_saved, digest_float_saved,
        "Digests should match for 1 and 1.0"
    );
}

#[tokio::test]
async fn test_manifest_digest_big_int_normalization() {
    let store_a = Arc::new(InMemory::new());
    let (registry_a, key_a, kid_a) = setup_registry_with_keys(store_a);

    // Test with i64::MAX and u64::MAX to ensure precision is kept
    let mut manifest_big = test_manifest("app_big_int", "1.0.0");
    manifest_big
        .configuration
        .insert("big_i64".to_string(), serde_json::json!(i64::MAX));
    manifest_big
        .configuration
        .insert("big_u64".to_string(), serde_json::json!(u64::MAX));
    manifest_big.security.manifest_signature_kid = kid_a;
    let digest_prefixed = compute_digest_prefixed(&manifest_big);
    manifest_big.security.manifest_signature = hex::encode(key_a.sign(digest_prefixed.as_bytes()).to_bytes());

    let (saved_digest, _) = registry_a.save_manifest(&manifest_big).await.unwrap();
    // SHA-384 hex is 96 chars
    assert_eq!(saved_digest.len(), 96);
}

#[tokio::test]
async fn test_get_artifact_returns_artifact_not_found_for_missing_key() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx()).unwrap();

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
    let registry = RegistryStorage::new(store, &test_tenant_ctx()).unwrap();

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
        // Test save_artifact
        let result = registry.save_artifact(key, b"data".as_slice().into()).await;
        match result {
            Err(RegistryError::InvalidPath(_)) => {}
            other => panic!("save_artifact: expected InvalidPath for {key}, got {other:?}"),
        }

        // Test save_manifest (id)
        let manifest = test_manifest(key, "1.0.0");
        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::InvalidPath(_)) => {}
            other => panic!("save_manifest: expected InvalidPath for {key} (id), got {other:?}"),
        }
    }
}

#[tokio::test]
async fn test_save_manifest_rejects_whitespace_security_fields() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx()).unwrap();

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
    manifest.security.manifest_signature = "sig".to_string(); // Need valid signature field to fail on kid check
    manifest.security.manifest_signature_kid = "   ".to_string();
    let result = registry.save_manifest(&manifest).await;
    match result {
        Err(RegistryError::InvalidManifest(msg)) => {
            assert_eq!(msg, "security.manifest_signature_kid must not be empty");
        }
        other => panic!("expected InvalidManifest for whitespace kid, got {other:?}"),
    }

    let mut manifest = test_manifest("app_whitespace_trust", "1.0.0");
    manifest.security.manifest_signature = "sig".to_string();
    manifest.security.manifest_signature_kid = "kid".to_string();
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

#[tokio::test]
async fn test_save_manifest_verifies_signature() {
     let store = Arc::new(InMemory::new());
    let (registry, signing_key, kid) = setup_registry_with_keys(store);

    let mut manifest = test_manifest("app_sig_test", "1.0.0");
    manifest.security.manifest_signature_kid = kid;
    let digest_prefixed = compute_digest_prefixed(&manifest);

    // 1. Valid Signature
    let signature = hex::encode(signing_key.sign(digest_prefixed.as_bytes()).to_bytes());
    manifest.security.manifest_signature = signature;
    registry.save_manifest(&manifest).await.expect("Valid signature should pass");

    // 2. Invalid Signature (Tampered Payload)
    let mut manifest_bad = manifest.clone();
    manifest_bad.name = "Changed".to_string(); // Changes digest
    // Keep old signature
    let result = registry.save_manifest(&manifest_bad).await;
    match result {
        Err(RegistryError::InvalidManifest(msg)) => {
            assert!(msg.contains("Signature verification failed"), "Got: {}", msg);
        }
        other => panic!("Expected InvalidManifest, got {:?}", other),
    }

    // 3. Invalid Signature (Tampered Signature)
    let mut manifest_bad_sig = manifest.clone();
    manifest_bad_sig.security.manifest_signature = "deadbeef".to_string();
    let result = registry.save_manifest(&manifest_bad_sig).await;
    match result {
        Err(RegistryError::InvalidManifest(_)) => {}
        other => panic!("Expected InvalidManifest for bad sig, got {:?}", other),
    }
}

#[tokio::test]
async fn test_get_missing_manifest_returns_manifest_not_found() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx()).unwrap();

    let result = registry.get_manifest("missing_id", "0.0.0").await;
    match result {
        Err(RegistryError::ManifestNotFound(path)) => {
            assert_eq!(path, "missing_id/0.0.0");
        }
        other => panic!("expected ManifestNotFound, got {other:?}"),
    }
}
