use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "audit_logs", schema_name = "flexi")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String, // UUID v7
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: String, // Partition Key / RLS Scope
    pub actor_id: String, // User ID or API Key ID
    pub action: String,   // e.g., "auth.login", "entity.create"
    pub resource: String, // e.g., "user:123", "entity:456"
    pub details: Json,    // Context, diff, etc.
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTimeWithTimeZone,
    pub archived_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
