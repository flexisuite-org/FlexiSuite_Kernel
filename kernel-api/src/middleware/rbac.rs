//! RBAC Middleware for FlexiSuite Kernel
//!
//! This module provides Role-Based Access Control (RBAC) middleware for protecting routes.
//!
//! # Middleware Ordering Requirements (CRITICAL)
//!
//! The RBAC middleware has strict ordering dependencies that MUST be enforced:
//!
//! 1. **Authentication Middleware** (MUST run first)
//!    - Validates tokens and creates `TenantContext` with `tenant_id` and `user_id`
//!    - Injects `BearerToken` extension
//!    - Example: `extract_bearer_token_middleware`, `validate_token_middleware`
//!
//! 2. **load_permissions_middleware** (MUST run after authentication)
//!    - Depends on: `TenantContext` (with user_id), `BearerToken`
//!    - Loads user permissions from database
//!    - Injects `UserPermissions` extension
//!
//! 3. **require_permission** (MUST run after load_permissions_middleware)
//!    - Depends on: `UserPermissions`
//!    - Enforces specific permission requirements
//!
//! ## Runtime Safety
//!
//! Missing dependencies are detected at runtime and return `403 FORBIDDEN`.
//! However, misconfigured routes that skip authentication entirely will bypass
//! permission checks. Always verify middleware ordering in route configuration.
//!
//! ## Example Usage
//!
//! ```rust,ignore
//! use axum::{Router, routing::get, middleware::from_fn};
//! use kernel_api::middleware::{
//!     extract_bearer_token_middleware,
//!     validate_token_middleware,
//!     load_permissions_middleware,
//!     require_permission_layer,
//! };
//!
//! let app = Router::new()
//!     .route("/api/data", get(handler))
//!     // Step 3: Require specific permission
//!     .layer(from_fn(require_permission_layer("data:read")))
//!     // Step 2: Load permissions (depends on TenantContext + BearerToken)
//!     .layer(from_fn(load_permissions_middleware))
//!     // Step 1: Authentication (creates TenantContext + BearerToken)
//!     .layer(from_fn(validate_token_middleware))
//!     .layer(from_fn(extract_bearer_token_middleware));
//! ```
//!
use crate::middleware::BearerToken;
use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};
use kernel_core::auth::KeyManager;
use kernel_data::{AuthenticatedScoped, RBACRepository, TenantContext, with_tenant_tx};
use std::collections::HashSet;
use std::future::Future;
use tracing::{error, warn};

/// Validates that a v2 token has the required kid field.
/// Token format: v2:{kid}:{timestamp}:{nonce}:{tenant_id}:{signature}
/// REQ-TENANT-TOKEN-V2: kid MUST be required in v2 tokens.
fn validate_v2_token_kid(token: &str) -> Result<(), StatusCode> {
    let parts: Vec<&str> = token.split(':').collect();
    if parts.len() != 6 {
        error!(
            "Invalid v2 token format: expected 6 parts, got {}",
            parts.len()
        );
        return Err(StatusCode::FORBIDDEN);
    }

    // parts[0] = "v2", parts[1] = kid
    let kid = parts[1];
    if kid.trim().is_empty() {
        error!("v2 token missing required kid field (REQ-TENANT-TOKEN-V2)");
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(())
}
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
        .ok_or(StatusCode::FORBIDDEN)?;

    // Manually extract BearerToken to avoid 500 on missing extension
    let token_ext = req
        .extensions()
        .get::<BearerToken>()
        .ok_or(StatusCode::FORBIDDEN)?;
    let incoming_token = &token_ext.0;

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
        return Err(StatusCode::FORBIDDEN);
    }

    // Branching logic for tokens (Requirement 3)
    let mut use_incoming_as_db_token = false;
    if incoming_token.starts_with("v2:") {
        // REQ-TENANT-TOKEN-V2: Validate that v2 token has required kid field
        validate_v2_token_kid(incoming_token)?;
        use_incoming_as_db_token = true;
    }
    #[cfg(any(feature = "test-utils", feature = "enable_dev_auth"))]
    if incoming_token.starts_with("dev-token:") {
        use_incoming_as_db_token = true;
    }
    let db_token = if use_incoming_as_db_token {
        incoming_token.clone()
    } else {
        // Bridge non-v2 request credentials onto a DB-recognized tenant token.
        let ctx_for_closure = ctx.clone();
        match ctx
            .with_system_context(|sys_ctx| async move {
                KeyManager::generate_tenant_token(&sys_ctx, ctx_for_closure.tenant_id()).await
            })
            .await
        {
            Ok(Ok(t)) => t,
            Ok(Err(e)) => {
                // Log sanitized error
                match e {
                    kernel_core::auth::KeyManagerError::Unauthorized(_) => {
                        error!("Failed to generate DB session token: Unauthorized");
                        return Err(StatusCode::FORBIDDEN);
                    }
                    kernel_core::auth::KeyManagerError::NoActiveKey(t) => {
                        warn!(
                            "Failed to generate DB session token: No active key for type {}",
                            t
                        );
                        return Err(StatusCode::SERVICE_UNAVAILABLE);
                    }
                    kernel_core::auth::KeyManagerError::DbError(_) => {
                        error!("Failed to generate DB session token: Database error")
                    }
                    _ => error!("Failed to generate DB session token: Internal error"),
                }
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
            Err(e) => {
                error!(
                    "Failed to obtain system context for token generation: {}",
                    e
                );
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    };

    // Execute within a tenant-scoped transaction to ensure RLS is active.
    // `AuthenticatedScoped::try_from_scoped` derives user_id directly from `scoped`,
    // preventing any external user_id injection (user-impersonation-by-misuse).
    let permissions_result = with_tenant_tx(db, &ctx, &db_token, move |scoped| {
        Box::pin(async move {
            let auth_scoped = AuthenticatedScoped::try_from_scoped(scoped).map_err(|_| {
                kernel_data::DataError::TenantAuthorizationFailed(
                    "user_id missing in scoped context".to_string(),
                )
            })?;
            RBACRepository::get_user_permissions(&auth_scoped).await
        })
    })
    .await;
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
                    error!("Failed to fetch permissions: Validation error (programming error)");
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
                kernel_data::DataError::DbError(_) => {
                    error!("Failed to fetch permissions: Database error")
                }
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
) -> impl Clone
+ Fn(
    Request,
    Next,
) -> std::pin::Pin<Box<dyn Future<Output = Result<Response, StatusCode>> + Send>> {
    move |req, next| Box::pin(require_permission(permission, req, next))
}
