use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "entity_records")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String, // UUID v7
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: String, // Partition Key / RLS Scope
    pub entity_type: String, // "app", "component", "user_data", etc.
    pub schema_version: i32,
    pub content: Json, // The actual data
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
    pub version: i32, // Optimistic Concurrency Control
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
