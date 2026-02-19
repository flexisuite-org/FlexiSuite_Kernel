use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "group_roles", schema_name = "flexi")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(primary_key, auto_increment = false)]
    pub tenant_id: String,
    pub group_id: Uuid,
    pub role_id: Uuid,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::group::Entity",
        from = "(Column::GroupId, Column::TenantId)",
        to = "(super::group::Column::Id, super::group::Column::TenantId)",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    Group,
    #[sea_orm(
        belongs_to = "super::role::Entity",
        from = "(Column::RoleId, Column::TenantId)",
        to = "(super::role::Column::Id, super::role::Column::TenantId)",
        on_update = "NoAction",
        on_delete = "Cascade"
    )]
    Role,
}

impl Related<super::group::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Group.def()
    }
}

impl Related<super::role::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Role.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
