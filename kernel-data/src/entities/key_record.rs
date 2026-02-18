use sea_orm::entity::prelude::*;
use serde::Serialize;
use std::fmt;

#[derive(Clone, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "key_record", schema_name = "flexi")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub kid: String,
    pub key_type: String,  // 'hmac', 'paseto_public', 'paseto_private'
    pub algorithm: String, // 'HS256', 'Ed25519'
    #[sea_orm(column_type = "Blob", nullable)]
    pub secret_bytes: Option<Vec<u8>>,
    #[sea_orm(column_type = "Blob", nullable)]
    pub public_bytes: Option<Vec<u8>>,
    pub state: String, // 'active', 'next', 'retired', 'revoked'
    pub created_at: DateTimeWithTimeZone,
    pub activated_at: Option<DateTimeWithTimeZone>,
    pub retired_at: Option<DateTimeWithTimeZone>,
    pub revoked_at: Option<DateTimeWithTimeZone>,
    pub expires_at: Option<DateTimeWithTimeZone>,
}

impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Model")
            .field("kid", &self.kid)
            .field("key_type", &self.key_type)
            .field("algorithm", &self.algorithm)
            .field(
                "secret_bytes",
                &self.secret_bytes.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "public_bytes",
                &self.public_bytes.as_ref().map(|_| "[REDACTED]"),
            )
            .field("state", &self.state)
            .field("created_at", &self.created_at)
            .field("activated_at", &self.activated_at)
            .field("retired_at", &self.retired_at)
            .field("revoked_at", &self.revoked_at)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KeyRecordDto {
    pub kid: String,
    pub key_type: String,
    pub algorithm: String,
    pub public_bytes: Option<Vec<u8>>,
    pub state: String,
    pub created_at: DateTimeWithTimeZone,
    pub activated_at: Option<DateTimeWithTimeZone>,
    pub retired_at: Option<DateTimeWithTimeZone>,
    pub revoked_at: Option<DateTimeWithTimeZone>,
    pub expires_at: Option<DateTimeWithTimeZone>,
}

impl From<&Model> for KeyRecordDto {
    fn from(value: &Model) -> Self {
        Self {
            kid: value.kid.clone(),
            key_type: value.key_type.clone(),
            algorithm: value.algorithm.clone(),
            public_bytes: value.public_bytes.clone(),
            state: value.state.clone(),
            created_at: value.created_at.clone(),
            activated_at: value.activated_at.clone(),
            retired_at: value.retired_at.clone(),
            revoked_at: value.revoked_at.clone(),
            expires_at: value.expires_at.clone(),
        }
    }
}
