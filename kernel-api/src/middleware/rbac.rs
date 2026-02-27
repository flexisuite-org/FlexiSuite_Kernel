use axum::{
    extract::Request,
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
};
use kernel_core::auth::{KeyManager, SystemTenantContext};
use kernel_data::{RBACRepository, TenantContext, with_tenant_tx};
use std::collections::HashSet;
use std::future::Future;
use tracing::{error, warn};
use crate::middleware::BearerToken;

#[derive(Clone, Debug)]
pub struct UserPermissions {
    inner: HashSet<String>,
}

impl UserPermissions {
    pub fn new(permissions: HashSet<String>) -> Self {
        Self { inner: permissions }
    }

    pub fn has(&self, permission: &str) -> bool {
        self.inner.contains(permission)
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

    // Manually extract BearerToken to avoid 500 on missing extension
    let _token_ext = req
        .extensions()
        .get::<BearerToken>()
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

    // The incoming token might be V4 (PASETO) which is verified by auth_middleware.
    // However, the DB function `authorize_tenant` currently only accepts V2 (HMAC) tokens.
    // We bridge this gap by generating a short-lived V2 token for the DB session using a System Context.
    // In production, we assume kernel-api has access to the system keys (via DB connection).
    //
    // SECURITY: This is a deliberate privilege escalation.
    // We trust that `auth_middleware` has already validated the user's identity/token.
    // `KeyManager` uses system-level keys to mint a new token that satisfies the DB's `authorize_tenant` check.
    // This token is short-lived and strictly scoped to the already-authenticated `tenant_id`.
    let db_arc = ctx.db_arc().cloned().ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let sys_ctx = TenantContext::from(SystemTenantContext).with_db(db_arc);
    // TODO: optimization - cache this token in Redis to avoid signing overhead on every request.
    // See: https://github.com/flexisuite-org/FlexiSuite_Kernel/issues/70
    let db_token = match KeyManager::generate_tenant_token(&sys_ctx, ctx.tenant_id()).await {
        Ok(t) => t,
        Err(e) => {
             // Log sanitized error
             match e {
                 kernel_core::auth::KeyManagerError::Unauthorized(_) => {
                     error!("Failed to generate DB session token: Unauthorized");
                     return Err(StatusCode::FORBIDDEN);
                 }
                 kernel_core::auth::KeyManagerError::DbError(_) => error!("Failed to generate DB session token: Database error"),
                 _ => error!("Failed to generate DB session token: Internal error"),
             }
             return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    // Execute within a tenant-scoped transaction to ensure RLS is active.
    // RBACRepository now demands a TenantScoped connection.
    let ctx_clone = ctx.clone();
    let permissions_result = with_tenant_tx(db, &ctx, &db_token, move |scoped| {
        Box::pin(async move {
            RBACRepository::get_user_permissions(scoped, &ctx_clone).await
        })
    }).await;

    let permissions_list = match permissions_result {
        Ok(perms) => perms,
        Err(e) => {
            // Log sanitized error
            match e {
                kernel_data::DataError::TenantAuthorizationFailed(_) => {
                    error!("Failed to fetch permissions: Authorization failed");
                    return Err(StatusCode::FORBIDDEN);
                }
                kernel_data::DataError::ValidationError(_) => {
                    error!("Failed to fetch permissions: Validation error");
                    return Err(StatusCode::FORBIDDEN);
                }
                kernel_data::DataError::DbError(_) => error!("Failed to fetch permissions: Database error"),
                _ => error!("Failed to fetch permissions: Internal error"),
            }
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
        .ok_or(StatusCode::FORBIDDEN)?;

    if !permissions.has(permission) {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(next.run(req).await)
}

/// Factory for creating a permission requirement layer.
/// Usage: `.layer(middleware::from_fn(require_permission_layer("data:read")))`
pub fn require_permission_layer(
    permission: &'static str,
) -> impl Clone + Fn(Request, Next) -> std::pin::Pin<Box<dyn Future<Output = Result<Response, StatusCode>> + Send>> {
    move |req, next| {
        Box::pin(require_permission(permission, req, next))
    }
}
