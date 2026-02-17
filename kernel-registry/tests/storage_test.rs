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

    let manifest = DistManifest {
        schema_version: "1.0".to_string(),
        id: "app_test".to_string(),
        version: "1.0.0".to_string(),
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
            manifest_signature: "sig_...".to_string(),
            manifest_signature_kid: "key1".to_string(),
            trust_root_version: "v1".to_string(),
        },
    };

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
    let store = Arc::new(InMemory::new());
    let registry = RegistryStorage::new(store, &test_tenant_ctx());

    let manifest_a = DistManifest {
        schema_version: "1.0".to_string(),
        id: "app_security_digest".to_string(),
        version: "1.0.0".to_string(),
        kind: Kind::Composition,
        name: "Security Digest Test".to_string(),
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
            manifest_signature: "sig_A".to_string(),
            manifest_signature_kid: "kid_A".to_string(),
            trust_root_version: "v1".to_string(),
        },
    };

    let mut manifest_b = manifest_a.clone();
    // Intentionally keep `manifest_b` with the same id/version as `manifest_a`.
    // In RegistryStorage::save_manifest this overwrites the same manifest path,
    // and this test's goal is only to assert that digest computation ignores
    // Security-field differences between manifest_a and manifest_b.
    manifest_b.security.manifest_signature = "sig_B".to_string();
    manifest_b.security.manifest_signature_kid = "kid_B".to_string();
    manifest_b.security.trust_root_version = "v2".to_string();

    // These two save_manifest calls intentionally target the same RegistryStorage path.
    let (digest_a, _) = registry.save_manifest(&manifest_a).await.unwrap();
    let (digest_b, _) = registry.save_manifest(&manifest_b).await.unwrap();

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

    let manifest = DistManifest {
        schema_version: "1.0".to_string(),
        id: "app_invalid_security".to_string(),
        version: "1.0.0".to_string(),
        kind: Kind::Composition,
        name: "Invalid Security".to_string(),
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
            manifest_signature_kid: "key1".to_string(),
            trust_root_version: "v1".to_string(),
        },
    };

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

    let manifest = DistManifest {
        schema_version: "1.0".to_string(),
        id: "app_invalid_security_kid".to_string(),
        version: "1.0.0".to_string(),
        kind: Kind::Composition,
        name: "Invalid Security Kid".to_string(),
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
            manifest_signature: "sig_...".to_string(),
            manifest_signature_kid: "".to_string(),
            trust_root_version: "v1".to_string(),
        },
    };

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

    let manifest = DistManifest {
        schema_version: "1.0".to_string(),
        id: "app_invalid_trust_root_version".to_string(),
        version: "1.0.0".to_string(),
        kind: Kind::Composition,
        name: "Invalid Trust Root Version".to_string(),
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
            manifest_signature: "sig_...".to_string(),
            manifest_signature_kid: "key1".to_string(),
            trust_root_version: "".to_string(),
        },
    };

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

    let manifest = DistManifest {
        schema_version: "1.0".to_string(),
        id: "app_tamper_test".to_string(),
        version: "1.0.0".to_string(),
        kind: Kind::Composition,
        name: "Tamper Test".to_string(),
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
            manifest_signature: "sig_...".to_string(),
            manifest_signature_kid: "key1".to_string(),
            trust_root_version: "v1".to_string(),
        },
    };

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

    let manifest = DistManifest {
        schema_version: "1.0".to_string(),
        id: "app_\0_test".to_string(),
        version: "1.0.0".to_string(),
        kind: Kind::Composition,
        name: "Invalid Key".to_string(),
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
            manifest_signature: "sig".to_string(),
            manifest_signature_kid: "kid".to_string(),
            trust_root_version: "v1".to_string(),
        },
    };

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
