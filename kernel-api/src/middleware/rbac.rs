use axum::{
    extract::{Extension, Request},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use kernel_data::{RBACRepository, TenantContext, with_tenant_tx};
use std::collections::HashSet;
use tracing::{error, warn};
use crate::middleware::BearerToken;

#[derive(Clone, Debug)]
pub struct UserPermissions(HashSet<String>);

impl UserPermissions {
    pub fn new(permissions: HashSet<String>) -> Self {
        Self(permissions)
    }

    pub fn has(&self, permission: &str) -> bool {
        self.0.contains(permission)
    }
}

pub async fn load_permissions_middleware(
    Extension(token_ext): Extension<BearerToken>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let ctx = req
        .extensions()
        .get::<TenantContext>()
        .cloned()
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // We need a DB connection to fetch permissions
    let db = match ctx.db() {
        Ok(db) => db,
        Err(_) => {
            warn!("Database connection missing in TenantContext");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    // Note: user_id existence is checked inside RBACRepository::get_user_permissions
    // via TenantContext, but we can fast-fail here if needed.
    if ctx.user_id().is_none() {
        // Fail closed if user_id is missing (unauthenticated or service account not allowed here)
        warn!("User ID missing in context for RBAC protected route");
        return Err(StatusCode::UNAUTHORIZED);
    }

    let token = &token_ext.0;

    // Execute within a tenant-scoped transaction to ensure RLS is active.
    // RBACRepository now demands a TenantScoped connection.
    let ctx_clone = ctx.clone();
    let permissions_result = with_tenant_tx(db, &ctx, token, move |scoped| {
        Box::pin(async move {
            RBACRepository::get_user_permissions(scoped, &ctx_clone).await
        })
    }).await;

    let permissions_list = match permissions_result {
        Ok(perms) => perms,
        Err(e) => {
            error!(error = %e, "Failed to fetch permissions");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let mut permissions_set = HashSet::new();
    for p in permissions_list {
        // Permission format: "resource:action" (e.g., "data:read")
        let perm_str = format!("{}:{}", p.resource, p.action);
        permissions_set.insert(perm_str);
    }

    req.extensions_mut()
        .insert(UserPermissions::new(permissions_set));

    Ok(next.run(req).await)
}

pub async fn require_permission(
    permission: &'static str,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let permissions = req
        .extensions()
        .get::<UserPermissions>()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;

    if !permissions.has(permission) {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(next.run(req).await)
}
