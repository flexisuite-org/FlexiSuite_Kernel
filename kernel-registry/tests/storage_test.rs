use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use kernel_core::auth::{TenantContext, TenantId};
use kernel_registry::error::RegistryError;
use kernel_registry::model::{Dependencies, DistManifest, Kind, Route, Security};
use kernel_registry::storage::{normalize_kid, reload_trust_root_keys, RegistryStorage};
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::ObjectStore;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex};

// Global lock to serialize tests that modify process-global environment variables.
static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

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
            manifest_signature: "sig_default".to_string(),
            manifest_signature_kid: "kid_default".to_string(),
            trust_root_version: "v1".to_string(),
        },
    }
}

fn compute_digest(manifest: &DistManifest) -> String {
    RegistryStorage::manifest_payload_digest(manifest).expect("Digest computation failed")
}

struct TestKey {
    kid: String,
    signing_key: SigningKey,
    public_key_b64: String,
}

impl TestKey {
    fn new(kid: &str) -> Self {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = VerifyingKey::from(&signing_key);
        let public_key_bytes = verifying_key.as_bytes();
        let public_key_b64 = BASE64_URL_SAFE_NO_PAD.encode(public_key_bytes);
        Self {
            kid: kid.to_string(),
            signing_key,
            public_key_b64,
        }
    }

    fn sign(&self, manifest: &mut DistManifest) {
        manifest.security.manifest_signature_kid = self.kid.clone();
        let digest_hex = compute_digest(manifest);
        // Verify RAW digest bytes (SHA-384 output), not hex string bytes.
        let digest_bytes = hex::decode(digest_hex).expect("Valid hex");
        let signature = self.signing_key.sign(&digest_bytes);
        manifest.security.manifest_signature = BASE64_URL_SAFE_NO_PAD.encode(signature.to_bytes());
    }

    fn env_var_name(&self) -> String {
        let normalized = normalize_kid(&self.kid);
        format!("FLEXI_REGISTRY_TRUST_ROOT_KEY_B64URL_{}", normalized)
    }
}

/// Helper to run registry tests with a specific trust root key configured.
/// Handles environment variable setting, trust root reloading, and async runtime.
fn with_test_key<F>(key: &TestKey, test_fn: F)
where
    F: for<'a> FnOnce(&'a TestKey, RegistryStorage) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>,
{
    // Serialize execution to prevent env var races
    let _guard = ENV_LOCK.lock().unwrap();
    temp_env::with_var(key.env_var_name(), Some(&key.public_key_b64), || {
        reload_trust_root_keys();
        let store = Arc::new(InMemory::new());
        let registry = RegistryStorage::new(store, &test_tenant_ctx());

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(test_fn(key, registry));
    });
}

/// Helper to run registry tests with a specific trust root key UNSET.
fn with_test_key_unset<F>(key: &TestKey, test_fn: F)
where
    F: for<'a> FnOnce(&'a TestKey, RegistryStorage) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>,
{
    // Serialize execution to prevent env var races
    let _guard = ENV_LOCK.lock().unwrap();
    temp_env::with_var(key.env_var_name(), None::<&str>, || {
        reload_trust_root_keys();
        let store = Arc::new(InMemory::new());
        let registry = RegistryStorage::new(store, &test_tenant_ctx());

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(test_fn(key, registry));
    });
}

#[tokio::test]
async fn test_save_and_get_artifact() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

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

#[test]
fn test_save_and_get_manifest() {
    let key = TestKey::new("test-key-1");
    with_test_key(&key, |key, registry| Box::pin(async move {
        let mut manifest = test_manifest("app_test", "1.0.0");
        key.sign(&mut manifest);

        let (digest, persisted) = registry.save_manifest(&manifest).await.unwrap();
        assert_eq!(digest.len(), 96);
        assert_eq!(persisted.security.manifest_digest, digest);

        let retrieved = registry.get_manifest("app_test", "1.0.0").await.unwrap();
        assert_eq!(retrieved.security.manifest_digest, digest);
        assert_eq!(
            retrieved.security.manifest_signature,
            manifest.security.manifest_signature
        );
    }));
}

#[test]
fn test_manifest_digest_excludes_security_section() {
    let key = TestKey::new("test-key-digest");
    with_test_key(&key, |key, registry_a| Box::pin(async move {
        let mut manifest_a = test_manifest("app_security_digest", "1.0.0");
        manifest_a.name = "Security Digest Test".to_string();
        key.sign(&mut manifest_a);

        let mut manifest_b = manifest_a.clone();
        // Modify a security field. The digest (payload) should remain unchanged.
        manifest_b.security.trust_root_version = "v2".to_string();

        let (digest_a, _) = registry_a.save_manifest(&manifest_a).await.unwrap();
        // Saving manifest_b should produce the same digest since only security fields differ.
        let (digest_b, _) = registry_a.save_manifest(&manifest_b).await.unwrap();

        assert_eq!(digest_a, digest_b);
    }));
}

#[test]
fn test_save_manifest_rejects_invalid_signature() {
    let key = TestKey::new("test-key-invalid");
    with_test_key(&key, |key, registry| Box::pin(async move {
        let mut manifest = test_manifest("app_invalid_sig", "1.0.0");
        key.sign(&mut manifest);
        // Tamper with signature
        manifest.security.manifest_signature = BASE64_URL_SAFE_NO_PAD.encode(b"bad_sig");

        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::SignatureVerificationFailed(_)) => {}
            other => panic!("expected SignatureVerificationFailed, got {other:?}"),
        }
    }));
}

#[test]
fn test_save_manifest_rejects_unknown_kid() {
    let key = TestKey::new("test-key-unknown");
    with_test_key_unset(&key, |key, registry| Box::pin(async move {
        let mut manifest = test_manifest("app_unknown_kid", "1.0.0");
        key.sign(&mut manifest);

        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::KeyNotFound(_)) => {}
            other => panic!("expected KeyNotFound, got {other:?}"),
        }
    }));
}

#[test]
fn test_normalized_kid_collision_rejects_ambiguous_keys() {
    let _guard = ENV_LOCK.lock().unwrap();
    let key_a = TestKey::new("abc");
    let key_b = TestKey::new("ABC");
    let env_a = "FLEXI_REGISTRY_TRUST_ROOT_KEY_B64URL_abc";
    let env_b = "FLEXI_REGISTRY_TRUST_ROOT_KEY_B64URL_ABC";

    temp_env::with_vars(
        [
            (env_a, Some(key_a.public_key_b64.as_str())),
            (env_b, Some(key_b.public_key_b64.as_str())),
        ],
        || {
            reload_trust_root_keys();
            let store = Arc::new(InMemory::new());
            let registry = RegistryStorage::new(store, &test_tenant_ctx());
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let mut manifest = test_manifest("app_collision", "1.0.0");
                key_a.sign(&mut manifest);

                let result = registry.save_manifest(&manifest).await;
                match result {
                    Err(RegistryError::KeyNotFound(_)) => {}
                    other => panic!("expected KeyNotFound for normalized KID collision, got {other:?}"),
                }
            });
        },
    );
}

#[tokio::test]
async fn test_get_missing_manifest_returns_manifest_not_found() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

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
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

    // Basic validation runs before signature verification, so no need for keys here
    let mut manifest = test_manifest("app_invalid_security", "1.0.0");
    manifest.security.manifest_signature = "".to_string();

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
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

    let mut manifest = test_manifest("app_invalid_security_kid", "1.0.0");
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
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

    let mut manifest = test_manifest("app_invalid_trust_root_version", "1.0.0");
    manifest.security.trust_root_version = "".to_string();

    let result = registry.save_manifest(&manifest).await;
    match result {
        Err(RegistryError::InvalidManifest(msg)) => {
            assert_eq!(msg, "security.trust_root_version must not be empty");
        }
        other => panic!("expected InvalidManifest, got {other:?}"),
    }
}

#[test]
fn test_get_manifest_detects_tampered_stored_json() {
    let key = TestKey::new("test-key-tamper");

    // We can't easily use with_test_key helper because we need to tamper with the store.
    // So we replicate the setup but keeping access to the store.

    temp_env::with_var(key.env_var_name(), Some(&key.public_key_b64), || {
        reload_trust_root_keys();
        let store = Arc::new(InMemory::new());
        let tenant_ctx = test_tenant_ctx();
        let registry = RegistryStorage::new(store.clone(), &tenant_ctx);

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut manifest = test_manifest("app_tamper_test", "1.0.0");
            key.sign(&mut manifest);

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
        })
    });
}

#[test]
fn test_manifest_digest_numeric_normalization() {
    let key = TestKey::new("test-key-numeric");
    with_test_key(&key, |key, registry| Box::pin(async move {
        let mut manifest_int = test_manifest("app_numeric", "1.0.0");
        manifest_int
            .configuration
            .insert("count".to_string(), serde_json::json!(1));
        // Sign AFTER modification
        key.sign(&mut manifest_int);

        let mut manifest_float = test_manifest("app_numeric", "1.0.0");
        manifest_float
            .configuration
            .insert("count".to_string(), serde_json::json!(1.0));

        // Assert normalization matches BEFORE signing/saving
        assert_eq!(compute_digest(&manifest_int), compute_digest(&manifest_float));

        // Reuse same signature from int for float version
        manifest_float.security.manifest_signature = manifest_int.security.manifest_signature.clone();
        manifest_float.security.manifest_signature_kid = manifest_int.security.manifest_signature_kid.clone();

        let (digest_int, _) = registry.save_manifest(&manifest_int).await.unwrap();
        let (digest_float, _) = registry.save_manifest(&manifest_float).await.unwrap();

        assert_eq!(digest_int, digest_float);
    }));
}

#[test]
fn test_manifest_digest_big_int_normalization() {
    let key = TestKey::new("test-key-bigint");
    with_test_key(&key, |key, registry| Box::pin(async move {
        let mut manifest_big = test_manifest("app_big_int", "1.0.0");
        manifest_big
            .configuration
            .insert("big_i64".to_string(), serde_json::json!(i64::MAX));
        manifest_big
            .configuration
            .insert("big_u64".to_string(), serde_json::json!(u64::MAX));
        key.sign(&mut manifest_big);

        let (digest, _) = registry.save_manifest(&manifest_big).await.unwrap();
        assert_eq!(digest.len(), 96);
    }));
}

#[tokio::test]
async fn test_save_manifest_rejects_control_character_in_id() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

    let manifest = test_manifest("app_\0_test", "1.0.0");
    // Fails before signature check
    let result = registry.save_manifest(&manifest).await;
    match result {
        Err(RegistryError::InvalidPath(_)) => {}
        other => panic!("expected InvalidPath, got {other:?}"),
    }
}

#[tokio::test]
async fn test_save_manifest_rejects_whitespace_security_fields() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

    // These fail before signature check
    let mut manifest = test_manifest("app_whitespace_sig", "1.0.0");
    manifest.security.manifest_signature = "   ".to_string();
    let result = registry.save_manifest(&manifest).await;
    match result {
        Err(RegistryError::InvalidManifest(_)) => {}
        other => panic!("expected InvalidManifest, got {other:?}"),
    }

    let mut manifest = test_manifest("app_whitespace_kid", "1.0.0");
    manifest.security.manifest_signature_kid = "   ".to_string();
    let result = registry.save_manifest(&manifest).await;
    match result {
        Err(RegistryError::InvalidManifest(_)) => {}
        other => panic!("expected InvalidManifest, got {other:?}"),
    }

    let mut manifest = test_manifest("app_whitespace_trust", "1.0.0");
    manifest.security.trust_root_version = "   ".to_string();
    let result = registry.save_manifest(&manifest).await;
    match result {
        Err(RegistryError::InvalidManifest(_)) => {}
        other => panic!("expected InvalidManifest, got {other:?}"),
    }
}

#[tokio::test]
async fn test_registry_key_validation_invalid_paths() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

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
        // Should fail InvalidPath before signature check
        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::InvalidPath(_)) => {}
            other => panic!("save_manifest: expected InvalidPath for {key} (id), got {other:?}"),
        }
    }
}
