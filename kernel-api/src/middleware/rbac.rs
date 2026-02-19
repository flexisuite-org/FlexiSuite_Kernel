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
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let db = match ctx.db() {
        Ok(db) => db,
        Err(_) => {
            warn!("Database connection missing in TenantContext");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let user_id = match ctx.user_id() {
        Some(uid) => uid,
        None => {
            req.extensions_mut()
                .insert(UserPermissions(HashSet::new()));
            return Ok(next.run(req).await);
        }
    };

    let permissions_list =
        match RBACRepository::get_user_permissions(db, ctx.tenant_id().as_str(), user_id.as_str())
            .await
        {
            Ok(perms) => perms,
            Err(e) => {
                warn!("Failed to fetch permissions: {}", e);
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        };

    let mut permissions_set = HashSet::new();
    for p in permissions_list {
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
