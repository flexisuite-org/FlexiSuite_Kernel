use chrono::{Duration, Utc};
use kernel_data::auth_context::{TenantContext, TenantId};
use kernel_data::entities::key_record::{self, ActiveModel, KeyState, KeyType, Model};
use kernel_data::entities::prelude::KeyRecord;
use ring::{
    rand::{SecureRandom, SystemRandom},
    signature::{Ed25519KeyPair, KeyPair},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait,
    QueryFilter, QuerySelect, Set, Statement, TransactionTrait,
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
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
}

pub struct KeyManager;

impl KeyManager {
    fn ensure_system(ctx: &TenantContext) -> Result<(), KeyManagerError> {
        if !ctx.is_system() {
            return Err(KeyManagerError::Unauthorized(
                "Key management operation requires system tenant context".to_string(),
            ));
        }
        Ok(())
    }

    /// Rotates keys for all supported types.
    pub async fn rotate_keys(ctx: &TenantContext) -> Result<(), KeyManagerError> {
        Self::ensure_system(ctx)?;
        let db = ctx.db().map_err(sea_orm::DbErr::Custom)?;
        let key_types = vec![(KeyType::Hmac, "HS256"), (KeyType::PasetoPublic, "Ed25519")];

        for (k_type, alg) in key_types {
            Self::rotate_key_type(db, k_type, alg).await?;
        }

        Ok(())
    }

    /// Internal rotation logic using raw DatabaseConnection.
    async fn rotate_key_type(
        db: &DatabaseConnection,
        key_type: KeyType,
        alg: &str,
    ) -> Result<(), KeyManagerError> {
        let txn = db.begin().await?;
        let key_type_str = match &key_type {
            KeyType::Hmac => "hmac",
            KeyType::PasetoPublic => "paseto_public",
        };

        // Serialize rotation per key type across instances, even when no rows exist yet.
        txn.execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT pg_advisory_xact_lock(hashtext($1))",
            [key_type_str.into()],
        ))
        .await?;

        // 1. Get current Active key
        let active_key = KeyRecord::find()
            .filter(key_record::Column::KeyType.eq(key_type.clone()))
            .filter(key_record::Column::State.eq(KeyState::Active))
            .lock_exclusive()
            .one(&txn)
            .await?;

        let now = Utc::now();

        if let Some(active) = active_key {
            // Check if rotation is needed (e.g., > 30 days)
            let rotation_base = active.activated_at.unwrap_or(active.created_at);
            let rotation_threshold = rotation_base + Duration::days(30);
            if now >= rotation_threshold {
                // Rotate!

                // 1. Retire current Active
                let mut active_am: ActiveModel = active.into();
                active_am.state = Set(KeyState::Retired);
                active_am.retired_at = Set(Some(now.into()));
                active_am.update(&txn).await?;

                // 2. Promote Next to Active
                let next_key = KeyRecord::find()
                    .filter(key_record::Column::KeyType.eq(key_type.clone()))
                    .filter(key_record::Column::State.eq(KeyState::Next))
                    .lock_exclusive()
                    .one(&txn)
                    .await?;

                if let Some(next) = next_key {
                    let mut next_am: ActiveModel = next.into();
                    next_am.state = Set(KeyState::Active);
                    next_am.activated_at = Set(Some(now.into()));
                    next_am.update(&txn).await?;
                } else {
                    // Emergency: No next key. Create a new Active key immediately.
                    Self::create_key(&txn, key_type.clone(), alg, KeyState::Active).await?;
                }

                // 3. Create new Next key
                Self::create_key(&txn, key_type.clone(), alg, KeyState::Next).await?;
            } else {
                // Ensure Next key exists
                let next_key = KeyRecord::find()
                    .filter(key_record::Column::KeyType.eq(key_type.clone()))
                    .filter(key_record::Column::State.eq(KeyState::Next))
                    .lock_exclusive()
                    .one(&txn)
                    .await?;

                if next_key.is_none() {
                    Self::create_key(&txn, key_type.clone(), alg, KeyState::Next).await?;
                }
            }
        } else {
            // No active key. Initialize.
            Self::create_key(&txn, key_type.clone(), alg, KeyState::Active).await?;
            Self::create_key(&txn, key_type.clone(), alg, KeyState::Next).await?;
        }

        // Prune retired keys older than 48h (must match flexi.authorize_tenant acceptance window).
        let prune_threshold = now - Duration::hours(48);
        KeyRecord::delete_many()
            .filter(key_record::Column::KeyType.eq(key_type.clone()))
            .filter(key_record::Column::State.eq(KeyState::Retired))
            .filter(key_record::Column::RetiredAt.lt(prune_threshold))
            .exec(&txn)
            .await?;

        txn.commit().await?;
        Ok(())
    }

    async fn create_key(
        db: &impl sea_orm::ConnectionTrait,
        key_type: KeyType,
        alg: &str,
        state: KeyState,
    ) -> Result<Model, KeyManagerError> {
        let now = Utc::now();
        let prefix = match key_type {
            KeyType::Hmac => "hmac",
            KeyType::PasetoPublic => "paseto_public",
        };
        let kid = format!("{}-{}-{}", prefix, now.timestamp(), Uuid::now_v7());
        // Token format uses ':' as a delimiter; kid must never contain ':'.
        if kid.contains(':') {
            return Err(KeyManagerError::KeyGenError(
                "Generated key id contains invalid ':' delimiter".to_string(),
            ));
        }

        let (secret, public) = match key_type {
            KeyType::Hmac => {
                let rng = SystemRandom::new();
                let mut key = vec![0u8; 32];
                rng.fill(&mut key)
                    .map_err(|_| KeyManagerError::KeyGenError("Failed to fill HMAC key".into()))?;
                (Some(key), None)
            }
            KeyType::PasetoPublic => {
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
        };

        let active_model = ActiveModel {
            kid: Set(kid),
            key_type: Set(key_type.clone()),
            algorithm: Set(alg.to_string()),
            secret_bytes: Set(secret),
            public_bytes: Set(public),
            state: Set(state.clone()),
            created_at: Set(now.into()),
            activated_at: Set(if state == KeyState::Active {
                Some(now.into())
            } else {
                None
            }),
            retired_at: Set(None),
            revoked_at: Set(None),
            expires_at: Set(None), // Can set hard expiry if needed
        };

        active_model.insert(db).await.map_err(Into::into)
    }

    /// Gets the current active key for signing/verification.
    ///
    /// For HMAC signing keys, we always read the authoritative active key from DB
    /// to avoid stale process-local cache after cross-instance revocation.
    pub async fn get_active_key(
        ctx: &TenantContext,
        key_type: KeyType,
    ) -> Result<Model, KeyManagerError> {
        Self::ensure_system(ctx)?;
        Self::get_active_key_internal(ctx, key_type).await
    }

    async fn get_active_key_internal(
        ctx: &TenantContext,
        key_type: KeyType,
    ) -> Result<Model, KeyManagerError> {
        let db = ctx.db().map_err(sea_orm::DbErr::Custom)?;
        let key = KeyRecord::find()
            .filter(key_record::Column::KeyType.eq(key_type.clone()))
            .filter(key_record::Column::State.eq(KeyState::Active))
            .one(db)
            .await?;

        key.ok_or_else(|| KeyManagerError::NoActiveKey(format!("{:?}", key_type)))
    }

    /// Gets a specific key by KID (for verification).
    pub async fn get_key(ctx: &TenantContext, kid: &str) -> Result<Model, KeyManagerError> {
        Self::ensure_system(ctx)?;
        let db = ctx.db().map_err(sea_orm::DbErr::Custom)?;
        KeyRecord::find_by_id(kid)
            .one(db)
            .await?
            .ok_or_else(|| KeyManagerError::KeyNotFound(kid.to_string()))
    }

    /// Revokes a key immediately.
    pub async fn revoke_key(ctx: &TenantContext, kid: &str) -> Result<(), KeyManagerError> {
        Self::ensure_system(ctx)?;
        let db = ctx.db().map_err(sea_orm::DbErr::Custom)?;
        let txn = db.begin().await?;
        let key = KeyRecord::find_by_id(kid)
            .lock_exclusive()
            .one(&txn)
            .await?;
        let key = key.ok_or_else(|| KeyManagerError::KeyNotFound(kid.to_string()))?;

        let now = Utc::now();
        let key_type = key.key_type.clone();
        let algorithm = key.algorithm.clone();
        let was_active = key.state == KeyState::Active;

        let mut am: ActiveModel = key.into();
        am.state = Set(KeyState::Revoked);
        am.revoked_at = Set(Some(now.into()));
        am.update(&txn).await?;

        if was_active {
            let active_replacement = KeyRecord::find()
                .filter(key_record::Column::KeyType.eq(key_type.clone()))
                .filter(key_record::Column::State.eq(KeyState::Active))
                .lock_exclusive()
                .one(&txn)
                .await?;

            if active_replacement.is_none() {
                let next_key = KeyRecord::find()
                    .filter(key_record::Column::KeyType.eq(key_type.clone()))
                    .filter(key_record::Column::State.eq(KeyState::Next))
                    .lock_exclusive()
                    .one(&txn)
                    .await?;

                if let Some(next) = next_key {
                    let mut next_am: ActiveModel = next.into();
                    next_am.state = Set(KeyState::Active);
                    next_am.activated_at = Set(Some(now.into()));
                    next_am.update(&txn).await?;
                    Self::create_key(&txn, key_type.clone(), &algorithm, KeyState::Next).await?;
                } else {
                    Self::create_key(&txn, key_type.clone(), &algorithm, KeyState::Active).await?;
                    Self::create_key(&txn, key_type.clone(), &algorithm, KeyState::Next).await?;
                }
            }
        }

        txn.commit().await?;
        Ok(())
    }

    /// Generates a tenant token using the active HMAC key for the given `TenantId`.
    pub async fn generate_tenant_token(
        ctx: &TenantContext,
        tenant_id: &TenantId,
    ) -> Result<String, KeyManagerError> {
        // Enforce tenant isolation: only system context or the tenant themselves can generate a token.
        if !ctx.is_system() && ctx.tenant_id() != tenant_id {
            return Err(KeyManagerError::Unauthorized(format!(
                "Context tenant {} cannot generate token for tenant {}",
                ctx.tenant_id(),
                tenant_id
            )));
        }

        let active_key = Self::get_active_key_internal(ctx, KeyType::Hmac).await?;
        let secret = active_key.secret_bytes.ok_or_else(|| {
            KeyManagerError::KeyGenError("No secret bytes for HMAC key".to_string())
        })?;

        let now = Utc::now().timestamp();
        let nonce = Uuid::now_v7().to_string();
        let ver = "v2";
        let kid = active_key.kid;

        let msg = format!("{}:{}:{}:{}:{}", ver, kid, now, nonce, tenant_id.as_str());

        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &secret);
        let tag = ring::hmac::sign(&key, msg.as_bytes());
        let sig = hex::encode(tag.as_ref());

        Ok(format!(
            "{}:{}:{}:{}:{}:{}",
            ver,
            kid,
            now,
            nonce,
            tenant_id.as_str(),
            sig
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_data::auth_context::SystemTenantContext;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_generate_tenant_token_has_v2_format_with_kid() {
        let mocked_kid = "hmac-key-mocked-01";
        let mock_active_key = Model {
            kid: mocked_kid.to_string(),
            key_type: KeyType::Hmac,
            algorithm: "HS256".to_string(),
            secret_bytes: Some(vec![0x11; 32]),
            public_bytes: None,
            state: KeyState::Active,
            created_at: Utc::now().into(),
            activated_at: Some(Utc::now().into()),
            retired_at: None,
            revoked_at: None,
            expires_at: None,
        };
        let mock_db = MockDatabase::new(DatabaseBackend::Postgres)
            // Mock KeyManager::get_active_key_internal -> KeyRecord::find().one(db)
            .append_query_results(vec![vec![mock_active_key]])
            .into_connection();
        let ctx = TenantContext::from(SystemTenantContext).with_db(Arc::new(mock_db));
        let tenant_id = TenantId::new("tenant_001").expect("tenant_id should be valid");

        let token = KeyManager::generate_tenant_token(&ctx, &tenant_id)
            .await
            .expect("token generation should succeed");
        let parts: Vec<&str> = token.split(':').collect();
        assert_eq!(parts.len(), 6, "v2 token must have 6 colon-separated parts");
        assert_eq!(parts[0], "v2", "version must be v2");
        assert_eq!(parts[1], mocked_kid, "kid must match active key from mocked DB");
        assert!(
            parts[2].parse::<u64>().is_ok(),
            "timestamp must be valid epoch seconds"
        );
        assert!(
            uuid::Uuid::parse_str(parts[3]).is_ok(),
            "nonce must be a valid UUID"
        );
        assert_eq!(parts[4], tenant_id.as_str(), "tenant_id must match input");
        assert_eq!(
            parts[5].len(),
            64,
            "signature must be 64 hex chars (HMAC-SHA256)"
        );
        assert!(
            parts[5].chars().all(|c| c.is_ascii_hexdigit()),
            "signature must be hex-encoded"
        );
    }

    #[tokio::test]
    async fn test_generate_tenant_token_rejects_non_system_cross_tenant_request() {
        let mock_db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let ctx = TenantContext::new(
            TenantId::new("tenant_001").expect("tenant_id should be valid"),
            None,
        )
        .with_db(Arc::new(mock_db));
        let other_tenant_id = TenantId::new("tenant_002").expect("tenant_id should be valid");

        let result = KeyManager::generate_tenant_token(&ctx, &other_tenant_id).await;
        assert!(
            matches!(result, Err(KeyManagerError::Unauthorized(_))),
            "non-system context must be rejected for cross-tenant token generation"
        );
    }

    #[tokio::test]
    async fn test_generate_tenant_token_allows_non_system_same_tenant_request() {
        let mock_active_key = Model {
            kid: "hmac-key-same-tenant".to_string(),
            key_type: KeyType::Hmac,
            algorithm: "HS256".to_string(),
            secret_bytes: Some(vec![0x22; 32]),
            public_bytes: None,
            state: KeyState::Active,
            created_at: Utc::now().into(),
            activated_at: Some(Utc::now().into()),
            retired_at: None,
            revoked_at: None,
            expires_at: None,
        };
        let mock_db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![mock_active_key]])
            .into_connection();
        let tenant_id = TenantId::new("tenant_001").expect("tenant_id should be valid");
        let ctx = TenantContext::new(tenant_id.clone(), None).with_db(Arc::new(mock_db));

        let token = KeyManager::generate_tenant_token(&ctx, &tenant_id).await;
        assert!(token.is_ok(), "same-tenant non-system token request must succeed");
    }
}
