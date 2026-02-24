use chrono::Utc;
use crate::auth_context::TenantId;
use crate::entities::key_record::{self, ActiveModel, KeyState, KeyType};
use ring::rand::{SecureRandom, SystemRandom};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use uuid::Uuid;

/// Test helpers for authentication.
///
/// NOTE: These helpers perform system-scoped DB reads/writes without using `TenantContext`.
/// They are strictly test-only and must NOT be used in production code.
///
/// This is an explicit exception to the rule that all DB access must go through
/// `TenantContext` and `TenantId`.
pub struct TestAuth;

impl TestAuth {
    pub async fn init_keys(db: &DatabaseConnection) -> Result<(), Box<dyn std::error::Error>> {
        let rng = SystemRandom::new();
        let mut key = vec![0u8; 32];
        rng.fill(&mut key).map_err(|_| "Failed to fill HMAC key")?;

        let kid = format!("hmac-{}-{}", Utc::now().timestamp(), Uuid::now_v7());

        let active_model = ActiveModel {
            kid: Set(kid.clone()),
            key_type: Set(KeyType::Hmac),
            algorithm: Set("HS256".to_string()),
            secret_bytes: Set(Some(key)),
            public_bytes: Set(None),
            state: Set(KeyState::Active),
            created_at: Set(Utc::now().into()),
            activated_at: Set(Some(Utc::now().into())),
            retired_at: Set(None),
            revoked_at: Set(None),
            expires_at: Set(None),
        };

        active_model.insert(db).await?;
        Ok(())
    }

    /// Helper that mirrors `KeyManager::generate_tenant_token` (v2:kid:ts:nonce:tenant_id:sig).
    ///
    /// TODO: Ideally this should call `KeyManager::generate_tenant_token` directly from kernel-core.
    /// However, adding kernel-core as a dev-dependency of kernel-data creates a circular dependency
    /// (kernel-data → kernel-core → kernel-data). Until this cycle is resolved (e.g., by extracting
    /// shared token logic into a separate crate), this helper must be kept in sync with
    /// `KeyManager::generate_tenant_token` manually.
    /// Tracking Issue: `docs/tech_debt/issue_kernel_token_extraction.md`
    ///
    /// WARNING: This logic duplicates `KeyManager` implementation. If the token format or signing
    /// logic changes in `KeyManager`, this MUST be updated to match.
    /// See: `docs/tech_debt/issue_kernel_token_extraction.md`
    pub async fn generate_tenant_token(
        db: &DatabaseConnection,
        tenant_id: &TenantId,
    ) -> Result<String, Box<dyn std::error::Error>> {
        use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};

        let key_record = key_record::Entity::find()
            .filter(key_record::Column::KeyType.eq(KeyType::Hmac))
            .filter(key_record::Column::State.eq(KeyState::Active))
            .order_by_desc(key_record::Column::ActivatedAt)
            .one(db)
            .await?
            .ok_or("No active HMAC key found")?;

        let secret = key_record.secret_bytes.ok_or("No secret bytes")?;
        let now = Utc::now().timestamp();
        let nonce = Uuid::now_v7().to_string();
        let ver = "v2";
        let kid = key_record.kid;

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
