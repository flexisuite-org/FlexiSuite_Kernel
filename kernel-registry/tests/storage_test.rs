use kernel_core::auth::{TenantContext, TenantId};
use kernel_registry::error::RegistryError;
use kernel_registry::model::{Dependencies, DistManifest, Kind, Route, Security};
use kernel_registry::storage::RegistryStorage;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::ObjectStore;
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

#[tokio::test]
async fn test_save_and_get_artifact() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

    let data = b"hello world";
    let digest = registry
        .save_artifact("test/file.txt", data.as_slice().into())
        .await
        .unwrap();

    // Check if digest is correct (SHA-384 of "hello world")
    // echo -n "hello world" | openssl dgst -sha384
    // (sha384) = fdbd8e75a67f29f701a4e040385e2e23986303ea10239211af907fcbb83578b3e417cb71ce646efd0819dd8c088de1bd
    assert_eq!(
        digest,
        "fdbd8e75a67f29f701a4e040385e2e23986303ea10239211af907fcbb83578b3e417cb71ce646efd0819dd8c088de1bd"
    );

    // Get with correct digest
    let retrieved = registry
        .get_artifact("test/file.txt", Some(&digest))
        .await
        .unwrap();
    assert_eq!(retrieved, data.as_slice());

    // Get with incorrect digest
    let result = registry
        .get_artifact("test/file.txt", Some("bad_digest"))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_save_and_get_manifest() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

    let mut manifest = test_manifest("app_test", "1.0.0");
    manifest.security.manifest_signature = "sig_...".to_string();
    manifest.security.manifest_signature_kid = "key1".to_string();

    let (digest, persisted) = registry.save_manifest(&manifest).await.unwrap();
    assert_eq!(digest.len(), 96); // SHA-384 hex string length
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
    let registry_a = RegistryStorage::new(store_a, &test_tenant_ctx());

    let store_b = Arc::new(InMemory::new());
    let registry_b = RegistryStorage::new(store_b, &test_tenant_ctx());

    let mut manifest_a = test_manifest("app_security_digest", "1.0.0");
    manifest_a.name = "Security Digest Test".to_string();
    manifest_a.security.manifest_signature = "sig_A".to_string();
    manifest_a.security.manifest_signature_kid = "kid_A".to_string();

    let mut manifest_b = manifest_a.clone();
    // Intentionally keep `manifest_b` with the same id/version/payload as `manifest_a`.
    // We utilize separate registry stores (`registry_a` and `registry_b`) to avoid logic relying on overwrite behavior,
    // while ensuring both digests are computed from the same payload despite differing security fields.
    manifest_b.security.manifest_signature = "sig_B".to_string();
    manifest_b.security.manifest_signature_kid = "kid_B".to_string();
    manifest_b.security.trust_root_version = "v2".to_string();

    let (digest_a, _) = registry_a.save_manifest(&manifest_a).await.unwrap();
    let (digest_b, _) = registry_b.save_manifest(&manifest_b).await.unwrap();

    assert_eq!(digest_a, digest_b);
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
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

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
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

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
    let registry = RegistryStorage::new(store.clone(), &tenant_ctx);

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
    store.put(&tampered_path, tampered_bytes.into()).await.unwrap();

    let result = registry.get_manifest(&manifest.id, &manifest.version).await;
    match result {
        Err(RegistryError::IntegrityCheckFailed { .. }) => {}
        other => panic!("expected IntegrityCheckFailed, got {other:?}"),
    }
}

#[tokio::test]
async fn test_save_manifest_rejects_control_character_in_id() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

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
    let registry_a = RegistryStorage::new(store_a, &test_tenant_ctx());

    let store_b = Arc::new(InMemory::new());
    let registry_b = RegistryStorage::new(store_b, &test_tenant_ctx());

    // Both must use SAME ID/Version/etc for digest to match
    let mut manifest_int = test_manifest("app_numeric", "1.0.0");
    manifest_int.configuration.insert("count".to_string(), serde_json::json!(1));

    let mut manifest_float = test_manifest("app_numeric", "1.0.0");
    manifest_float.configuration.insert("count".to_string(), serde_json::json!(1.0));

    // Ensure our input assumption is correct: json!(1) != json!(1.0) usually in serde_json Value representation
    // (though partial_eq might say they are equal, their serialization might differ without normalization)
    // Actually, serde_json::to_vec(json!(1)) -> "1", to_vec(json!(1.0)) -> "1.0". 
    // We want to ensure the digests are identical.

    let (digest_int, _) = registry_a.save_manifest(&manifest_int).await.unwrap();
    let (digest_float, _) = registry_b.save_manifest(&manifest_float).await.unwrap();

    assert_eq!(digest_int, digest_float, "Digests should match for 1 and 1.0");
}

#[tokio::test]
async fn test_manifest_digest_big_int_normalization() {
    let store_a = Arc::new(InMemory::new());
    let registry_a = RegistryStorage::new(store_a, &test_tenant_ctx());

    // Test with i64::MAX and u64::MAX to ensure precision is kept
    let mut manifest_big = test_manifest("app_big_int", "1.0.0");
    manifest_big.configuration.insert("big_i64".to_string(), serde_json::json!(i64::MAX));
    manifest_big.configuration.insert("big_u64".to_string(), serde_json::json!(u64::MAX));

    let (digest, _) = registry_a.save_manifest(&manifest_big).await.unwrap();
    assert_eq!(digest.len(), 96);
}

#[tokio::test]
async fn test_get_artifact_returns_artifact_not_found_for_missing_key() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

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
    let registry = RegistryStorage::new(store, &test_tenant_ctx());
    
    let invalid_keys = vec![
        "../traversal",
        "key\\backslash",
        "encode%2f",
        "encode%5c",
    ];

    for key in invalid_keys {
        // Test save_artifact
        let result = registry.save_artifact(key, b"data".as_slice().into()).await;
        match result {
            Err(RegistryError::InvalidPath(_)) => {},
            other => panic!("save_artifact: expected InvalidPath for {key}, got {other:?}"),
        }
        
        // Test save_manifest (id)
        let manifest = test_manifest(key, "1.0.0");
        let result = registry.save_manifest(&manifest).await;
        match result {
            Err(RegistryError::InvalidPath(_)) => {},
            other => panic!("save_manifest: expected InvalidPath for {key} (id), got {other:?}"),
        }
    }
}

#[tokio::test]
async fn test_save_manifest_rejects_whitespace_security_fields() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

    let mut manifest = test_manifest("app_whitespace_sig", "1.0.0");
    manifest.security.manifest_signature = "   ".to_string();
    let result = registry.save_manifest(&manifest).await;
    match result {
        Err(RegistryError::InvalidManifest(msg)) => {
            assert_eq!(msg, "security.manifest_signature must not be empty");
        },
        other => panic!("expected InvalidManifest for whitespace signature, got {other:?}"),
    }

    let mut manifest = test_manifest("app_whitespace_kid", "1.0.0");
    manifest.security.manifest_signature_kid = "   ".to_string();
    let result = registry.save_manifest(&manifest).await;
    match result {
        Err(RegistryError::InvalidManifest(msg)) => {
            assert_eq!(msg, "security.manifest_signature_kid must not be empty");
        },
        other => panic!("expected InvalidManifest for whitespace kid, got {other:?}"),
    }
    
    let mut manifest = test_manifest("app_whitespace_trust", "1.0.0");
    manifest.security.trust_root_version = "   ".to_string();
    let result = registry.save_manifest(&manifest).await;
    match result {
        Err(RegistryError::InvalidManifest(msg)) => {
            assert_eq!(msg, "security.trust_root_version must not be empty");
        },
        other => panic!("expected InvalidManifest for whitespace trust_root_version, got {other:?}"),
    }
}
