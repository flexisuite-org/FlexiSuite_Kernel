use crate::entities::{group, group_member, group_role, permission, role};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, JoinType, QueryFilter, RelationTrait, QuerySelect,
};

pub struct RBACRepository;

impl RBACRepository {
    pub async fn get_user_permissions(
        db: &impl ConnectionTrait,
        tenant_id: &str,
        user_id: &str,
    ) -> Result<Vec<permission::Model>, DbErr> {
        let permissions = permission::Entity::find()
            .filter(permission::Column::TenantId.eq(tenant_id))
            .join(JoinType::InnerJoin, permission::Relation::Role.def())
            .join(JoinType::InnerJoin, role::Relation::GroupRoles.def())
            .join(JoinType::InnerJoin, group_role::Relation::Group.def())
            .join(JoinType::InnerJoin, group::Relation::GroupMembers.def())
            .filter(group_member::Column::UserId.eq(user_id))
            .all(db)
            .await?;

        Ok(permissions)
    }
}
