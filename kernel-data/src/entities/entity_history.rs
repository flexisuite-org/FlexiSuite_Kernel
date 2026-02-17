use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "entity_histories", schema_name = "flexi")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String, // UUID v7
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: String, // Partition Key / RLS Scope
    pub entity_id: String, // FK to entity_records.id
    pub entity_type: String,
    pub change_type: String, // "CREATE", "UPDATE", "DELETE"
    pub version: i32,        // Snapshot version
    pub diff: Json,          // The changes or full snapshot
    pub created_at: DateTimeWithTimeZone,
    pub created_by: Option<String>, // User ID or System
    pub archived_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::entity_record::Entity",
        from = "Column::EntityId",
        to = "super::entity_record::Column::Id",
        on_update = "NoAction",
        on_delete = "NoAction"
    )]
    EntityRecord,
}

impl Related<super::entity_record::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::EntityRecord.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
