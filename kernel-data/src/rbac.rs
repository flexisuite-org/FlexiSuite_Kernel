use crate::connection::AuthenticatedScoped;
use crate::entities::{group, group_member, group_role, permission, role};
use crate::error::DataError;
use sea_orm::{ColumnTrait, EntityTrait, JoinType, QueryFilter, QuerySelect, RelationTrait};

pub struct RBACRepository;

impl RBACRepository {
    // TODO(perf): Add Redis caching for this query. This is a hot path (called on every request)
    // and involves a 5-table join. Tracking issue: https://github.com/flexisuite-org/FlexiSuite_Kernel/issues/99
    pub async fn get_user_permissions(
        scoped: &AuthenticatedScoped<'_>,
    ) -> Result<Vec<permission::Model>, DataError> {
        // Tenant isolation is guaranteed structurally:
        //   1. The underlying transaction was opened via `with_tenant_tx`, which calls
        //      `flexi.authorize_tenant` and sets the RLS context for the session.
        //   2. `AuthenticatedScoped` is constructed only after the authentication middleware
        //      has already confirmed that `user_id` is present in the `TenantContext`.
        //
        // Defense-in-depth: we additionally filter every table in the 5-table JOIN by
        // `tenant_id` so that a misconfigured RLS policy can never leak cross-tenant rows.

        let tenant_id = scoped.tenant_id().as_str();
        let user_id = scoped.user_id().as_str();

        let db = scoped.txn();

        let permissions = permission::Entity::find()
            .filter(permission::Column::TenantId.eq(tenant_id))
            .join(JoinType::InnerJoin, permission::Relation::Role.def())
            .filter(role::Column::TenantId.eq(tenant_id))
            .join(JoinType::InnerJoin, role::Relation::GroupRoles.def())
            .filter(group_role::Column::TenantId.eq(tenant_id))
            .join(JoinType::InnerJoin, group_role::Relation::Group.def())
            .filter(group::Column::TenantId.eq(tenant_id))
            .join(JoinType::InnerJoin, group::Relation::GroupMembers.def())
            .filter(group_member::Column::TenantId.eq(tenant_id))
            .filter(group_member::Column::UserId.eq(user_id))
            .all(db)
            .await
            .map_err(DataError::DbError)?;

        Ok(permissions)
    }
}
