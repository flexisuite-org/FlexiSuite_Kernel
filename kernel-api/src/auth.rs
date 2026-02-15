use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};

#[derive(Clone, Debug)]
pub struct TenantContext {
    pub tenant_id: String,
    pub user_id: Option<String>,
}

/// REQ-AUTH-SOURCE: Extract TenantContext from token or dev-headers (if debug)
pub async fn auth_middleware(mut req: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    let auth_header = req.headers().get("Authorization");
    
    let context = if let Some(header) = auth_header {
        // Real logic: Decode PASETO/JWT
        // Mock logic: bearer-<tenant_id>
        let val = header.to_str().map_err(|_| StatusCode::UNAUTHORIZED)?;
        if val.starts_with("Bearer ") {
            let token = &val[7..];
            if token.is_empty() { return Err(StatusCode::UNAUTHORIZED); }
            
            TenantContext {
                tenant_id: token.to_string(),
                user_id: Some("user-123".to_string()),
            }
        } else {
            return Err(StatusCode::UNAUTHORIZED);
        }
    } else {
        // REQ-AUTH-SOURCE: dev_only fallback
        #[cfg(debug_assertions)]
        {
            if let Some(tenant_id) = req.headers().get("X-Tenant-Id") {
                let id = tenant_id.to_str().map_err(|_| StatusCode::FORBIDDEN)?;
                TenantContext {
                    tenant_id: id.to_string(),
                    user_id: None,
                }
            } else {
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
        
        #[cfg(not(debug_assertions))]
        {
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    // Store in request extensions
    req.extensions_mut().insert(context);
    
    Ok(next.run(req).await)
}
