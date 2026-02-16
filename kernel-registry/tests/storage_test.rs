use kernel_registry::model::{DistManifest, Security, Dependencies, Route};
use kernel_registry::storage::RegistryStorage;
use object_store::memory::InMemory;
use std::sync::Arc;
use std::collections::HashMap;

#[tokio::test]
async fn test_save_and_get_artifact() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store);

    let data = b"hello world";
    let digest = registry.save_artifact("test/file.txt", data.as_slice().into()).await.unwrap();

    // Check if digest is correct (SHA-384 of "hello world")
    // echo -n "hello world" | openssl dgst -sha384
    // (sha384) = fdbd8e75a67f29f701a4e040385e2e23986303ea10239211af907fcbb83578b3e417cb71ce646efd0819dd8c088de1bd
    assert_eq!(digest, "fdbd8e75a67f29f701a4e040385e2e23986303ea10239211af907fcbb83578b3e417cb71ce646efd0819dd8c088de1bd");

    // Get with correct digest
    let retrieved = registry.get_artifact("test/file.txt", Some(&digest)).await.unwrap();
    assert_eq!(retrieved, data.as_slice());

    // Get with incorrect digest
    let result = registry.get_artifact("test/file.txt", Some("bad_digest")).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_save_and_get_manifest() {
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store);

    let manifest = DistManifest {
        schema_version: "1.0".to_string(),
        id: "app_test".to_string(),
        version: "1.0.0".to_string(),
        kind: "composition".to_string(),
        name: "Test App".to_string(),
        protected: false,
        composition_root: "main.tsx".to_string(),
        routes: vec![Route { path: "/".to_string(), component: "layout".to_string() }],
        dependencies: Dependencies {
            components: HashMap::new(),
            permissions: vec![],
        },
        configuration: HashMap::new(),
        security: Security {
            manifest_digest: "sha384-...".to_string(),
            manifest_signature: "sig_...".to_string(),
            manifest_signature_kid: "key1".to_string(),
            trust_root_version: "v1".to_string(),
        },
    };

    let digest = registry.save_manifest(&manifest).await.unwrap();
    assert_eq!(digest.len(), 96); // SHA-384 hex string length

    let retrieved = registry.get_manifest("app_test", "1.0.0").await.unwrap();
    assert_eq!(retrieved, manifest);
}
