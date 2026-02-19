use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "groups", schema_name = "flexi")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: String,
    pub name: String,
    pub description: String,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::group_member::Entity")]
    GroupMembers,
    #[sea_orm(has_many = "super::group_role::Entity")]
    GroupRoles,
}

impl Related<super::group_member::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::GroupMembers.def()
    }
}

impl Related<super::group_role::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::GroupRoles.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
