use sea_orm::entity::prelude::*;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "key_record", schema_name = "flexi")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub kid: String,
    pub key_type: String, // 'hmac', 'paseto_public', 'paseto_private'
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
