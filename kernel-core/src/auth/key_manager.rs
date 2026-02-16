use chrono::{Duration, Utc};
use kernel_data::entities::key_record::{self, ActiveModel, Model};
use kernel_data::entities::prelude::KeyRecord;
use rand::{rngs::OsRng, RngCore};
use ring::{
    rand::SystemRandom,
    signature::{Ed25519KeyPair, KeyPair},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum KeyManagerError {
    #[error("Database error: {0}")]
    DbError(#[from] sea_orm::DbErr),
    #[error("Key generation error: {0}")]
    KeyGenError(String),
    #[error("No active key found for type {0}")]
    NoActiveKey(String),
    #[error("Key not found: {0}")]
    KeyNotFound(String),
}

pub struct KeyManager;

impl KeyManager {
    /// Rotates keys for all supported types.
    pub async fn rotate_keys(db: &DatabaseConnection) -> Result<(), KeyManagerError> {
        let key_types = vec![
            ("hmac", "HS256"),
            ("paseto_public", "Ed25519"),
        ];

        for (k_type, alg) in key_types {
            Self::rotate_key_type(db, k_type, alg).await?;
        }

        Ok(())
    }

    async fn rotate_key_type(
        db: &DatabaseConnection,
        key_type: &str,
        alg: &str,
    ) -> Result<(), KeyManagerError> {
        let txn = db.begin().await?;

        // 1. Get current Active key
        let active_key = KeyRecord::find()
            .filter(key_record::Column::KeyType.eq(key_type))
            .filter(key_record::Column::State.eq("active"))
            .one(&txn)
            .await?;

        let now = Utc::now();

        if let Some(active) = active_key {
            // Check if rotation is needed (e.g., > 30 days)
            let rotation_threshold = active.created_at + Duration::days(30);
            if now >= rotation_threshold {
                // Rotate!

                // 1. Retire current Active
                let mut active_am: ActiveModel = active.into();
                active_am.state = Set("retired".to_string());
                active_am.retired_at = Set(Some(now.into()));
                active_am.update(&txn).await?;

                // 2. Promote Next to Active
                let next_key = KeyRecord::find()
                    .filter(key_record::Column::KeyType.eq(key_type))
                    .filter(key_record::Column::State.eq("next"))
                    .one(&txn)
                    .await?;

                if let Some(next) = next_key {
                    let mut next_am: ActiveModel = next.into();
                    next_am.state = Set("active".to_string());
                    next_am.activated_at = Set(Some(now.into()));
                    next_am.update(&txn).await?;
                } else {
                    // Emergency: No next key. Create a new Active key immediately.
                    Self::create_key(&txn, key_type, alg, "active").await?;
                }

                // 3. Create new Next key
                Self::create_key(&txn, key_type, alg, "next").await?;
            } else {
                // Ensure Next key exists
                 let next_key = KeyRecord::find()
                    .filter(key_record::Column::KeyType.eq(key_type))
                    .filter(key_record::Column::State.eq("next"))
                    .one(&txn)
                    .await?;

                if next_key.is_none() {
                     Self::create_key(&txn, key_type, alg, "next").await?;
                }
            }
        } else {
            // No active key. Initialize.
            Self::create_key(&txn, key_type, alg, "active").await?;
            Self::create_key(&txn, key_type, alg, "next").await?;
        }

        // Prune retired keys older than 24h
        let prune_threshold = now - Duration::hours(24);
        KeyRecord::delete_many()
            .filter(key_record::Column::KeyType.eq(key_type))
            .filter(key_record::Column::State.eq("retired"))
            .filter(key_record::Column::RetiredAt.lt(prune_threshold))
            .exec(&txn)
            .await?;

        txn.commit().await?;
        Ok(())
    }

    async fn create_key(
        db: &impl sea_orm::ConnectionTrait,
        key_type: &str,
        alg: &str,
        state: &str,
    ) -> Result<Model, KeyManagerError> {
        let kid = format!("{}-{}-{}", key_type, Utc::now().timestamp(), Uuid::now_v7());

        let (secret, public) = match key_type {
            "hmac" => {
                let mut key = vec![0u8; 32];
                OsRng.fill_bytes(&mut key);
                (Some(key), None)
            }
            "paseto_public" => {
                let rng = SystemRandom::new();
                let pkcs8_bytes = Ed25519KeyPair::generate_pkcs8(&rng)
                    .map_err(|e| KeyManagerError::KeyGenError(e.to_string()))?;
                let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8_bytes.as_ref())
                    .map_err(|e| KeyManagerError::KeyGenError(e.to_string()))?;

                // We store the keypair (pkcs8) as secret, and public key as public.
                // ring's Ed25519KeyPair doesn't expose private bytes easily, but pkcs8 is the portable format.
                let pub_key = key_pair.public_key().as_ref().to_vec();
                (Some(pkcs8_bytes.as_ref().to_vec()), Some(pub_key))
            }
            _ => return Err(KeyManagerError::KeyGenError(format!("Unsupported key type: {}", key_type))),
        };

        let active_model = ActiveModel {
            kid: Set(kid),
            key_type: Set(key_type.to_string()),
            algorithm: Set(alg.to_string()),
            secret_bytes: Set(secret),
            public_bytes: Set(public),
            state: Set(state.to_string()),
            created_at: Set(Utc::now().into()),
            activated_at: Set(if state == "active" { Some(Utc::now().into()) } else { None }),
            retired_at: Set(None),
            revoked_at: Set(None),
            expires_at: Set(None), // Can set hard expiry if needed
        };

        let res = active_model.insert(db).await?;
        Ok(res)
    }

    /// Gets the current active key for signing.
    pub async fn get_active_key(
        db: &DatabaseConnection,
        key_type: &str,
    ) -> Result<Model, KeyManagerError> {
        let key = KeyRecord::find()
            .filter(key_record::Column::KeyType.eq(key_type))
            .filter(key_record::Column::State.eq("active"))
            .one(db)
            .await?;

        key.ok_or_else(|| KeyManagerError::NoActiveKey(key_type.to_string()))
    }

    /// Gets a specific key by KID (for verification).
    pub async fn get_key(
        db: &DatabaseConnection,
        kid: &str,
    ) -> Result<Model, KeyManagerError> {
        KeyRecord::find_by_id(kid)
            .one(db)
            .await?
            .ok_or_else(|| KeyManagerError::KeyNotFound(kid.to_string()))
    }

    /// Revokes a key immediately.
    pub async fn revoke_key(
        db: &DatabaseConnection,
        kid: &str,
    ) -> Result<(), KeyManagerError> {
        let key = KeyRecord::find_by_id(kid).one(db).await?;
        if let Some(key) = key {
            let mut am: ActiveModel = key.into();
            am.state = Set("revoked".to_string());
            am.revoked_at = Set(Some(Utc::now().into()));
            am.update(db).await?;
        }
        Ok(())
    }

    /// Generates a tenant token using the active HMAC key.
    pub async fn generate_tenant_token(
        db: &DatabaseConnection,
        tenant_id: &str,
    ) -> Result<String, KeyManagerError> {
        let active_key = Self::get_active_key(db, "hmac").await?;
        let secret = active_key.secret_bytes.ok_or_else(|| KeyManagerError::KeyGenError("No secret bytes for HMAC key".to_string()))?;

        let now = Utc::now().timestamp();
        let nonce = Uuid::now_v7().to_string();
        let ver = "v2";
        let kid = active_key.kid;

        let msg = format!("{}:{}:{}:{}:{}", ver, kid, now, nonce, tenant_id);

        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &secret);
        let tag = ring::hmac::sign(&key, msg.as_bytes());
        let sig = hex::encode(tag.as_ref());

        Ok(format!("{}:{}:{}:{}:{}:{}", ver, kid, now, nonce, tenant_id, sig))
    }
}
