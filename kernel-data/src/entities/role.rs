use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "roles", schema_name = "flexi")]
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
    #[sea_orm(
        has_many = "super::permission::Entity",
        from = "(Column::Id, Column::TenantId)",
        to = "(super::permission::Column::RoleId, super::permission::Column::TenantId)"
    )]
    Permissions,
    #[sea_orm(
        has_many = "super::group_role::Entity",
        from = "(Column::Id, Column::TenantId)",
        to = "(super::group_role::Column::RoleId, super::group_role::Column::TenantId)"
    )]
    GroupRoles,
}

impl Related<super::permission::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Permissions.def()
    }
}

impl Related<super::group_role::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::GroupRoles.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
