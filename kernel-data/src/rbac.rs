use crate::entities::{group, group_member, group_role, permission, role};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, JoinType, QueryFilter, RelationTrait, QuerySelect,
};

pub struct RBACRepository;

impl RBACRepository {
    // TODO: Add Redis caching for this query. This is a hot path (called on every request)
    // and involves a 5-table join.
    pub async fn get_user_permissions(
        ctx: &crate::auth_context::TenantContext,
    ) -> Result<Vec<permission::Model>, DbErr> {
        let db = ctx.db().map_err(|e| DbErr::Custom(e))?;
        let tenant_id = ctx.tenant_id().as_str();
        let user_id = ctx.user_id().map(|u| u.as_str()).unwrap_or("");

        // Set RLS context if token is available
        if let Some(token) = ctx.db_token() {
            db.execute(sea_orm::Statement::from_sql_and_values(
                sea_orm::DbBackend::Postgres,
                "SELECT flexi.authorize_tenant($1)",
                [token.into()],
            ))
            .await?;
        }

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
