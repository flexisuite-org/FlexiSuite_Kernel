use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use kernel_data::{RBACRepository, TenantContext};
use std::collections::HashSet;
use tracing::warn;

#[derive(Clone, Debug)]
pub struct UserPermissions(pub HashSet<String>);

impl UserPermissions {
    pub fn has(&self, permission: &str) -> bool {
        self.0.contains(permission)
    }
}

pub async fn load_permissions_middleware(
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let ctx = req
        .extensions()
        .get::<TenantContext>()
        .cloned()
        .ok_or_else(|| {
            StatusCode::UNAUTHORIZED
        })?;

    match ctx.db() {
        Ok(_) => { }
        Err(_) => {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    if ctx.user_id().is_none() {
        req.extensions_mut()
            .insert(UserPermissions(HashSet::new()));
        return Ok(next.run(req).await);
    }

    // Generate a temporary tenant token for RLS authorization
    let token = match kernel_core::auth::KeyManager::generate_tenant_token(&ctx, ctx.tenant_id()).await {
        Ok(t) => t,
        Err(e) => {
            warn!("Failed to generate tenant token for RBAC: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let ctx_with_token = ctx.with_token(token);

    let permissions_list = match RBACRepository::get_user_permissions(&ctx_with_token).await {
        Ok(perms) => perms,
        Err(e) => {
            warn!("Failed to fetch permissions: {}", e);
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
        .insert(UserPermissions(permissions_set));

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
