use crate::error::RegistryError;
use kernel_core::supplychain::{KeyStatus, TrustedKey};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info, warn};

#[derive(Debug, Serialize, Deserialize)]
pub struct TrustRoot {
    pub version: String,
    pub generated_at: String,
    pub keys: Vec<TrustRootKey>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TrustRootKey {
    pub kid: String,
    pub alg: String,
    pub public_key: String,
    pub status: String,
    pub retired_at: Option<u64>,
    pub not_before: Option<u64>,
    pub not_after: Option<u64>,
}

pub trait TrustProvider: Send + Sync {
    fn get_key(&self, kid: &str) -> Result<TrustedKey, RegistryError>;
    fn trust_root_version(&self) -> &str;
}

pub struct FileTrustProvider {
    trust_root: Arc<TrustRoot>,
}

impl FileTrustProvider {
    pub fn new(path: PathBuf) -> Result<Self, RegistryError> {
        info!("Loading trust root from {:?}", path);
        let content = fs::read_to_string(&path).map_err(|e| {
            error!("Failed to read trust root at {:?}: {}", path, e);
            RegistryError::TrustRootError(format!("Failed to read trust root: {}", e))
        })?;

        let root: TrustRoot = serde_json::from_str(&content).map_err(|e| {
            error!("Failed to parse trust root: {}", e);
            RegistryError::TrustRootError(format!("Failed to parse trust root: {}", e))
        })?;

        Ok(Self {
            trust_root: Arc::new(root),
        })
    }
}

impl TrustProvider for FileTrustProvider {
    fn get_key(&self, kid: &str) -> Result<TrustedKey, RegistryError> {
        let key = self
            .trust_root
            .keys
            .iter()
            .find(|k| k.kid == kid)
            .ok_or_else(|| {
                warn!("Key not found in trust root: {}", kid);
                RegistryError::TrustRootError(format!("Key not found: {}", kid))
            })?;

        let status = match key.status.as_str() {
            "active" => KeyStatus::Active,
            "next" => KeyStatus::Next,
            "retired" => KeyStatus::Retired,
            "revoked" => KeyStatus::Revoked,
            _ => {
                warn!("Unknown key status: {}", key.status);
                return Err(RegistryError::TrustRootError(format!(
                    "Unknown key status: {}",
                    key.status
                )));
            }
        };

        let mut public_key = [0u8; 32];
        if key.alg != "Ed25519" {
            return Err(RegistryError::TrustRootError(format!(
                "Unsupported algorithm for key {}: {}",
                key.kid, key.alg
            )));
        }
        hex::decode_to_slice(&key.public_key, &mut public_key).map_err(|e| {
            RegistryError::TrustRootError(format!("Invalid hex in public key {}: {}", key.kid, e))
        })?;

        Ok(TrustedKey {
            kid: key.kid.clone(),
            alg: key.alg.clone(),
            status,
            retired_at: key.retired_at,
            not_before: key.not_before,
            not_after: key.not_after,
            public_key,
        })
    }

    fn trust_root_version(&self) -> &str {
        &self.trust_root.version
    }
}

#[cfg(any(test, feature = "test-utils"))]
pub mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Clone)]
    pub struct MockTrustProvider {
        keys: HashMap<String, TrustedKey>,
        trust_root_version: String,
    }

    impl MockTrustProvider {
        pub fn new() -> Self {
            Self::with_version("v1")
        }

        pub fn with_version(version: impl Into<String>) -> Self {
            Self {
                keys: HashMap::new(),
                trust_root_version: version.into(),
            }
        }

        pub fn add_key(&mut self, key: TrustedKey) {
            self.keys.insert(key.kid.clone(), key);
        }
    }

    impl TrustProvider for MockTrustProvider {
        fn get_key(&self, kid: &str) -> Result<TrustedKey, RegistryError> {
            self.keys
                .get(kid)
                .cloned()
                .ok_or_else(|| RegistryError::TrustRootError(format!("Key not found: {}", kid)))
        }

        fn trust_root_version(&self) -> &str {
            &self.trust_root_version
        }
    }
}
