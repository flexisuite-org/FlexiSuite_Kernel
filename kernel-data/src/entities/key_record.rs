use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Deserialize, Serialize)]
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
