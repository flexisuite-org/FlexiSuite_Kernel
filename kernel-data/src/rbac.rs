use crate::connection::AuthenticatedScoped;
use crate::entities::{group, group_member, group_role, permission, role};
use crate::error::DataError;
use sea_orm::{ColumnTrait, EntityTrait, JoinType, QueryFilter, QuerySelect, RelationTrait};

pub struct RBACRepository;

impl RBACRepository {
    // TODO(perf): Add Redis caching for this query. This is a hot path (called on every request)
    // and involves a 5-table join. Tracking issue: https://github.com/flexisuite-org/FlexiSuite_Kernel/issues/99
    pub async fn get_user_permissions(
        auth_scoped: &AuthenticatedScoped<'_>,
    ) -> Result<Vec<permission::Model>, DataError> {
        // Tenant isolation is guaranteed structurally:
        //   1. The underlying transaction was opened via `with_tenant_tx`, which calls
        //      `flexi.authorize_tenant` and sets the RLS context for the session.
        //   2. `AuthenticatedScoped` is constructed only after the authentication middleware
        //      has already confirmed that `user_id` is present in the `TenantContext`.
        //
        // Defense-in-depth: we additionally filter every table in the 5-table JOIN by
        // `tenant_id` so that a misconfigured RLS policy can never leak cross-tenant rows.

        let tenant_id = auth_scoped.tenant_id().as_str();
        let user_id = auth_scoped.user_id().as_str();
        let db = auth_scoped.txn();

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
            .distinct()
            .all(db)
            .await
            .map_err(DataError::DbError)?;

        Ok(permissions)
    }
}

#[cfg(feature = "test-utils")]
pub async fn seed_rbac_membership_for_tests(
    scoped: &crate::connection::TenantScoped<crate::connection::RawConnection>,
    tenant_id: &crate::auth_context::TenantId,
    user_id: &crate::auth_context::UserId,
) -> Result<(), DataError> {
    use sea_orm::{ActiveModelTrait, ActiveValue};
    use uuid::Uuid;

    let now = chrono::Utc::now().into();

    let role = role::ActiveModel {
        id: ActiveValue::Set(Uuid::now_v7()),
        tenant_id: ActiveValue::Set(tenant_id.to_string()),
        name: ActiveValue::Set("role-1".to_string()),
        description: ActiveValue::Set("desc".to_string()),
        created_at: ActiveValue::Set(now),
        updated_at: ActiveValue::Set(now),
    };
    let role = role
        .insert(&scoped.inner.txn)
        .await
        .map_err(DataError::DbError)?;

    let perm = permission::ActiveModel {
        id: ActiveValue::Set(Uuid::now_v7()),
        tenant_id: ActiveValue::Set(tenant_id.to_string()),
        role_id: ActiveValue::Set(role.id),
        resource: ActiveValue::Set("res-1".to_string()),
        action: ActiveValue::Set("act-1".to_string()),
        created_at: ActiveValue::Set(now),
        updated_at: ActiveValue::Set(now),
    };
    perm.insert(&scoped.inner.txn)
        .await
        .map_err(DataError::DbError)?;

    let group = group::ActiveModel {
        id: ActiveValue::Set(Uuid::now_v7()),
        tenant_id: ActiveValue::Set(tenant_id.to_string()),
        name: ActiveValue::Set("group-1".to_string()),
        description: ActiveValue::Set("desc".to_string()),
        created_at: ActiveValue::Set(now),
        updated_at: ActiveValue::Set(now),
    };
    let group = group
        .insert(&scoped.inner.txn)
        .await
        .map_err(DataError::DbError)?;

    let gr = group_role::ActiveModel {
        id: ActiveValue::Set(Uuid::now_v7()),
        group_id: ActiveValue::Set(group.id),
        role_id: ActiveValue::Set(role.id),
        tenant_id: ActiveValue::Set(tenant_id.to_string()),
        created_at: ActiveValue::Set(now),
        updated_at: ActiveValue::Set(now),
    };
    gr.insert(&scoped.inner.txn)
        .await
        .map_err(DataError::DbError)?;

    let gm = group_member::ActiveModel {
        id: ActiveValue::Set(Uuid::now_v7()),
        group_id: ActiveValue::Set(group.id),
        user_id: ActiveValue::Set(user_id.to_string()),
        tenant_id: ActiveValue::Set(tenant_id.to_string()),
        created_at: ActiveValue::Set(now),
        updated_at: ActiveValue::Set(now),
    };
    gm.insert(&scoped.inner.txn)
        .await
        .map_err(DataError::DbError)?;

    Ok(())
}
