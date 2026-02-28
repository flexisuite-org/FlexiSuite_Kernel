use sea_orm::entity::prelude::*;
use serde::Serialize;
use std::fmt;

#[derive(Clone, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "key_record", schema_name = "flexi")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub kid: String,
    pub key_type: KeyType,
    pub algorithm: String, // 'HS256', 'Ed25519'
    #[sea_orm(column_type = "Blob", nullable)]
    pub secret_bytes: Option<Vec<u8>>,
    #[sea_orm(column_type = "Blob", nullable)]
    pub public_bytes: Option<Vec<u8>>,
    pub state: KeyState,
    pub created_at: DateTimeWithTimeZone,
    pub activated_at: Option<DateTimeWithTimeZone>,
    pub retired_at: Option<DateTimeWithTimeZone>,
    pub revoked_at: Option<DateTimeWithTimeZone>,
    pub expires_at: Option<DateTimeWithTimeZone>,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum KeyType {
    #[sea_orm(string_value = "hmac")]
    Hmac,
    #[sea_orm(string_value = "paseto_public")]
    PasetoPublic,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum KeyState {
    #[sea_orm(string_value = "active")]
    Active,
    #[sea_orm(string_value = "next")]
    Next,
    #[sea_orm(string_value = "retired")]
    Retired,
    #[sea_orm(string_value = "revoked")]
    Revoked,
}

impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Model")
            .field("kid", &self.kid)
            .field("key_type", &self.key_type)
            .field("algorithm", &self.algorithm)
            .field("secret_bytes_present", &self.secret_bytes.is_some())
            .field("public_bytes_present", &self.public_bytes.is_some())
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
            key_type: value.key_type.to_value(),
            algorithm: value.algorithm.clone(),
            public_bytes: value.public_bytes.clone(),
            state: value.state.to_value(),
            created_at: value.created_at,
            activated_at: value.activated_at,
            retired_at: value.retired_at,
            revoked_at: value.revoked_at,
            expires_at: value.expires_at,
        }
    }
}
