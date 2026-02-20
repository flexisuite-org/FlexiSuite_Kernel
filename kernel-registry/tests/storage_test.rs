use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use kernel_core::auth::{TenantContext, TenantId};
use kernel_registry::error::RegistryError;
use kernel_registry::model::{Dependencies, DistManifest, Kind, Route, Security};
use kernel_registry::storage::RegistryStorage;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::ObjectStore;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::Serialize;
use sha2::{Digest, Sha384};
use std::collections::BTreeMap;
use std::sync::Arc;

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

// Copy of ManifestDigestPayload to ensure tests match implementation logic
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TestManifestDigestPayload<'a> {
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

fn compute_digest(manifest: &DistManifest) -> String {
    let payload = TestManifestDigestPayload {
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
    let payload_value = serde_json::to_value(&payload).unwrap();
    let normalized = normalize_value(payload_value);
    let payload_bytes = serde_json::to_vec(&normalized).unwrap();
    let mut hasher = Sha384::new();
    hasher.update(payload_bytes);
    hex::encode(hasher.finalize())
}

struct TestKey {
    kid: String,
    key_pair: Ed25519KeyPair,
    public_key_b64: String,
}

impl TestKey {
    fn new(kid: &str) -> Self {
        let rng = SystemRandom::new();
        let pkcs8_bytes = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8_bytes.as_ref()).unwrap();
        let public_key_bytes = key_pair.public_key().as_ref();
        let public_key_b64 = BASE64_URL_SAFE_NO_PAD.encode(public_key_bytes);
        Self {
            kid: kid.to_string(),
            key_pair,
            public_key_b64,
        }
    }

    fn sign(&self, manifest: &mut DistManifest) {
        manifest.security.manifest_signature_kid = self.kid.clone();
        let digest = compute_digest(manifest);
        let signature = self.key_pair.sign(digest.as_bytes());
        manifest.security.manifest_signature = BASE64_URL_SAFE_NO_PAD.encode(signature.as_ref());
    }

    fn env_var_name(&self) -> String {
        let normalized = self
            .kid
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();
        format!("FLEXI_REGISTRY_TRUST_ROOT_KEY_B64URL_{}", normalized)
    }
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
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());
    let key = TestKey::new("test-key-1");

    temp_env::with_var(key.env_var_name(), Some(&key.public_key_b64), || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
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
        })
    });
}

#[test]
fn test_manifest_digest_excludes_security_section() {
    let store_a = Arc::new(InMemory::new());
    let registry_a = RegistryStorage::new(store_a, &test_tenant_ctx());

    let store_b = Arc::new(InMemory::new());
    let registry_b = RegistryStorage::new(store_b, &test_tenant_ctx());

    let key = TestKey::new("test-key-digest");

    temp_env::with_var(key.env_var_name(), Some(&key.public_key_b64), || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut manifest_a = test_manifest("app_security_digest", "1.0.0");
            manifest_a.name = "Security Digest Test".to_string();
            key.sign(&mut manifest_a);

            let mut manifest_b = manifest_a.clone();
            manifest_b.security.trust_root_version = "v2".to_string();

            let (digest_a, _) = registry_a.save_manifest(&manifest_a).await.unwrap();
            let (digest_b, _) = registry_b.save_manifest(&manifest_b).await.unwrap();

            assert_eq!(digest_a, digest_b);
        })
    });
}

#[test]
fn test_save_manifest_rejects_invalid_signature() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());
    let key = TestKey::new("test-key-invalid");

    temp_env::with_var(key.env_var_name(), Some(&key.public_key_b64), || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut manifest = test_manifest("app_invalid_sig", "1.0.0");
            key.sign(&mut manifest);
            // Tamper with signature
            manifest.security.manifest_signature = BASE64_URL_SAFE_NO_PAD.encode(b"bad_sig");

            let result = registry.save_manifest(&manifest).await;
            match result {
                Err(RegistryError::SignatureVerificationFailed(_)) => {}
                other => panic!("expected SignatureVerificationFailed, got {other:?}"),
            }
        })
    });
}

#[tokio::test]
async fn test_save_manifest_rejects_unknown_kid() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());
    let key = TestKey::new("test-key-unknown");

    // Do NOT set env var
    let mut manifest = test_manifest("app_unknown_kid", "1.0.0");
    key.sign(&mut manifest);

    let result = registry.save_manifest(&manifest).await;
    match result {
        Err(RegistryError::KeyNotFound(_)) => {}
        other => panic!("expected KeyNotFound, got {other:?}"),
    }
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
    let store = Arc::new(InMemory::new());
    let tenant_ctx = test_tenant_ctx();
    let registry = RegistryStorage::new(store.clone(), &tenant_ctx);
    let key = TestKey::new("test-key-tamper");

    temp_env::with_var(key.env_var_name(), Some(&key.public_key_b64), || {
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
    let store_a = Arc::new(InMemory::new());
    let registry_a = RegistryStorage::new(store_a, &test_tenant_ctx());

    let store_b = Arc::new(InMemory::new());
    let registry_b = RegistryStorage::new(store_b, &test_tenant_ctx());

    let key = TestKey::new("test-key-numeric");

    temp_env::with_var(key.env_var_name(), Some(&key.public_key_b64), || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
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
            // Reuse same signature? No, because json!(1.0) might affect digest if not normalized.
            // But we test that digest IS normalized.
            // If normalized, payloads are identical.
            // So we can use the signature from manifest_int for manifest_float?
            // Yes, if digest logic works.
            manifest_float.security.manifest_signature = manifest_int.security.manifest_signature.clone();
            manifest_float.security.manifest_signature_kid = manifest_int.security.manifest_signature_kid.clone();

            let (digest_int, _) = registry_a.save_manifest(&manifest_int).await.unwrap();
            let (digest_float, _) = registry_b.save_manifest(&manifest_float).await.unwrap();

            assert_eq!(digest_int, digest_float);
        })
    });
}

#[test]
fn test_manifest_digest_big_int_normalization() {
    let store_a = Arc::new(InMemory::new());
    let registry_a = RegistryStorage::new(store_a, &test_tenant_ctx());
    let key = TestKey::new("test-key-bigint");

    temp_env::with_var(key.env_var_name(), Some(&key.public_key_b64), || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut manifest_big = test_manifest("app_big_int", "1.0.0");
            manifest_big
                .configuration
                .insert("big_i64".to_string(), serde_json::json!(i64::MAX));
            manifest_big
                .configuration
                .insert("big_u64".to_string(), serde_json::json!(u64::MAX));
            key.sign(&mut manifest_big);

            let (digest, _) = registry_a.save_manifest(&manifest_big).await.unwrap();
            assert_eq!(digest.len(), 96);
        })
    });
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
