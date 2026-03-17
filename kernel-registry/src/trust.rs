use crate::error::RegistryError;
use kernel_core::supplychain::{KeyStatus, TrustedKey};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

const CLOCK_DRIFT_TOLERANCE_SECONDS: u64 = 30;

#[derive(Debug, Serialize, Deserialize)]
pub struct TrustRoot {
    pub version: String,
    pub generated_at: String,
    pub keys: Vec<TrustRootKey>,
}

fn current_unix_ts() -> Result<u64, RegistryError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| RegistryError::TrustRootError(format!("System clock error: {e}")))
        .map(|d| d.as_secs())
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
    fn get_key(&self, kid: &str) -> Result<TrustedKey, RegistryError> {
        let now = current_unix_ts()?;
        self.get_key_at(kid, now)
    }
    fn get_key_at(&self, kid: &str, now: u64) -> Result<TrustedKey, RegistryError>;
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
    fn get_key_at(&self, kid: &str, now: u64) -> Result<TrustedKey, RegistryError> {
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
            "revoked" => {
                warn!("Revoked key requested from trust root: {}", key.kid);
                return Err(RegistryError::KeyRevoked {
                    kid: key.kid.clone(),
                });
            }
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

        if let Some(not_before) = key.not_before
            && now.saturating_add(CLOCK_DRIFT_TOLERANCE_SECONDS) < not_before
        {
            warn!(
                kid = %key.kid,
                now,
                not_before,
                "Key is not yet valid"
            );
            return Err(RegistryError::KeyNotYetValid {
                kid: key.kid.clone(),
                valid_from: not_before,
                now,
            });
        }
        if let Some(not_after) = key.not_after
            && now > not_after.saturating_add(CLOCK_DRIFT_TOLERANCE_SECONDS)
        {
            warn!(
                kid = %key.kid,
                now,
                not_after,
                "Key has expired"
            );
            return Err(RegistryError::KeyExpired {
                kid: key.kid.clone(),
                expired_at: not_after,
                now,
            });
        }

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
    /// Lightweight `TrustProvider` test double.
    ///
    /// This mock intentionally does not enforce temporal validation (`not_before` / `not_after`)
    /// in `get_key_at`; it only resolves by `kid`. Time-window behavior is covered by
    /// `FileTrustProvider` tests in this module.
    pub struct MockTrustProvider {
        keys: HashMap<String, TrustedKey>,
        trust_root_version: String,
    }

    impl MockTrustProvider {
        #[allow(clippy::new_without_default)]
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
        fn get_key_at(&self, kid: &str, _now: u64) -> Result<TrustedKey, RegistryError> {
            // Intentionally ignore `_now` to keep this test double focused on key lookup behavior.
            self.keys
                .get(kid)
                .cloned()
                .ok_or_else(|| RegistryError::TrustRootError(format!("Key not found: {}", kid)))
        }

        fn trust_root_version(&self) -> &str {
            &self.trust_root_version
        }
    }

    #[cfg(test)]
    fn write_trust_root_file(
        not_before: Option<u64>,
        not_after: Option<u64>,
    ) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().expect("create temp trust root");
        let trust_root = serde_json::json!({
            "version": "v1",
            "generated_at": "2026-03-04T00:00:00Z",
            "keys": [
                {
                    "kid": "kid-test",
                    "alg": "Ed25519",
                    "public_key": "0000000000000000000000000000000000000000000000000000000000000000",
                    "status": "active",
                    "retired_at": null,
                    "not_before": not_before,
                    "not_after": not_after
                }
            ]
        });
        std::fs::write(
            file.path(),
            serde_json::to_vec(&trust_root).expect("serialize trust root"),
        )
        .expect("write trust root");
        file
    }

    #[cfg(test)]
    #[test]
    fn file_trust_provider_accepts_not_before_within_clock_skew() {
        let now = 1_700_000_000u64;
        let file = write_trust_root_file(Some(now + CLOCK_DRIFT_TOLERANCE_SECONDS), None);
        let provider = FileTrustProvider::new(file.path().to_path_buf()).expect("load provider");
        let key = provider
            .get_key_at("kid-test", now)
            .expect("key should be accepted");
        assert_eq!(key.kid, "kid-test");
    }

    #[cfg(test)]
    #[test]
    fn file_trust_provider_rejects_not_before_beyond_clock_skew() {
        let now = 1_700_000_000u64;
        let file = write_trust_root_file(Some(now + CLOCK_DRIFT_TOLERANCE_SECONDS + 1), None);
        let provider = FileTrustProvider::new(file.path().to_path_buf()).expect("load provider");
        let err = provider
            .get_key_at("kid-test", now)
            .expect_err("key should be rejected");
        assert!(matches!(err, RegistryError::KeyNotYetValid { .. }));
    }

    #[cfg(test)]
    #[test]
    fn file_trust_provider_accepts_not_after_within_clock_skew() {
        let now = 1_700_000_000u64;
        let file = write_trust_root_file(None, Some(now - CLOCK_DRIFT_TOLERANCE_SECONDS));
        let provider = FileTrustProvider::new(file.path().to_path_buf()).expect("load provider");
        let key = provider
            .get_key_at("kid-test", now)
            .expect("key should be accepted");
        assert_eq!(key.kid, "kid-test");
    }

    #[cfg(test)]
    #[test]
    fn file_trust_provider_rejects_not_after_beyond_clock_skew() {
        let now = 1_700_000_000u64;
        let file = write_trust_root_file(None, Some(now - CLOCK_DRIFT_TOLERANCE_SECONDS - 1));
        let provider = FileTrustProvider::new(file.path().to_path_buf()).expect("load provider");
        let err = provider
            .get_key_at("kid-test", now)
            .expect_err("key should be rejected");
        assert!(matches!(err, RegistryError::KeyExpired { .. }));
    }
}
