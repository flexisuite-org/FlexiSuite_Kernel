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
}

pub struct FileTrustProvider {
    path: PathBuf,
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
            path,
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

        Ok(TrustedKey {
            kid: key.kid.clone(),
            alg: key.alg.clone(),
            public_key: key.public_key.clone(),
            status,
            retired_at: key.retired_at,
            not_before: key.not_before,
            not_after: key.not_after,
        })
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::HashMap;

    pub struct MockTrustProvider {
        keys: HashMap<String, TrustedKey>,
    }

    impl MockTrustProvider {
        pub fn new() -> Self {
            Self {
                keys: HashMap::new(),
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
    }
}
