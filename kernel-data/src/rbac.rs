use crate::auth_context::TenantContext;
use crate::connection::RawConnection;
use crate::connection::TenantScoped;
use crate::entities::{group, group_member, group_role, permission, role};
use crate::error::DataError;
use sea_orm::{ColumnTrait, EntityTrait, JoinType, QueryFilter, QuerySelect, RelationTrait};

pub struct RBACRepository;

impl RBACRepository {
    // TODO(perf): Add Redis caching for this query. This is a hot path (called on every request)
    // and involves a 5-table join. Tracking issue: https://github.com/flexisuite-org/FlexiSuite_Kernel/issues/99
    pub async fn get_user_permissions(
        scoped: &TenantScoped<RawConnection>,
        ctx: &TenantContext,
    ) -> Result<Vec<permission::Model>, DataError> {
        // Strict Tenant Isolation: Use the scoped transaction which ensures RLS context is set.
        // We also explicitly filter by tenant_id from the context for double safety.

        let tenant_id = ctx.tenant_id();
        let user_id = ctx
            .user_id()
            .ok_or_else(|| DataError::ValidationError("User ID missing in context".to_string()))?;

        // Sanity check: Ensure scoped context matches passed context
        if scoped.tenant_id != *tenant_id {
            return Err(DataError::TenantAuthorizationFailed(
                "Context mismatch in RBAC repository".to_string(),
            ));
        }

        let db = scoped.txn();

        let permissions = permission::Entity::find()
            .filter(permission::Column::TenantId.eq(tenant_id.as_str()))
            .join(JoinType::InnerJoin, permission::Relation::Role.def())
            .join(JoinType::InnerJoin, role::Relation::GroupRoles.def())
            .join(JoinType::InnerJoin, group_role::Relation::Group.def())
            .join(JoinType::InnerJoin, group::Relation::GroupMembers.def())
            .filter(group_member::Column::UserId.eq(user_id.as_str()))
            .all(db)
            .await
            .map_err(DataError::DbError)?;

        Ok(permissions)
    }
}
