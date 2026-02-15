use axum::{
    body::Body,
    http::{Request, StatusCode, Method},
    middleware::Next,
    response::{Response, IntoResponse},
};
use crate::auth::TenantContext;
use kernel_core::idempotency::{canonicalize_request_target, check_idempotency_conflict};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

// REQ-IDEMPOTENCY-STORE: Simulation of 24h storage
#[derive(Clone, Debug)]
pub struct IdempotencyRecord {
    pub body_hash: String,
    pub action_id: String,
    // ttl etc omitted for simulation
}

pub type IdempotencyStore = Arc<RwLock<HashMap<String, IdempotencyRecord>>>;

/// REQ-IDEMPOTENCY-HEADER: Middleware logic
pub async fn idempotency_middleware(req: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    // Only apply to write methods
    let method = req.method();
    if method == Method::GET || method == Method::HEAD || method == Method::OPTIONS {
        return Ok(next.run(req).await);
    }

    let idempotency_key = match req.headers().get("Idempotency-Key") {
        Some(val) => val.to_str().map_err(|_| StatusCode::BAD_REQUEST)?.to_string(),
        None => return Ok(next.run(req).await), // Optional header
    };

    let tenant_ctx = req.extensions().get::<TenantContext>().ok_or(StatusCode::UNAUTHORIZED)?;
    
    // Canonicalize
    let path = req.uri().path();
    let query = req.uri().query();
    let canonical_target = canonicalize_request_target(path, query);

    // Uniqueness Scope: (tenant_id, method, canonical_target, key)
    let scope_key = format!("{}:{}:{}:{}", tenant_ctx.tenant_id, method, canonical_target, idempotency_key);

    // Simulated Body Hash (Real implementation would buffer body)
    // For simulation, we assume some header or just fixed hash if missing
    let body_hash = req.headers().get("X-Mock-Body-Hash")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("hash-default")
        .to_string();

    // Check Store
    let store = req.extensions().get::<IdempotencyStore>()
        .cloned() // Clone Arc to release borrow of req
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    
    {
        let read_lock = store.read().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(record) = read_lock.get(&scope_key) {
            // REQ-IDEMPOTENCY-CONFLICT: Conflict Guard logic
            if check_idempotency_conflict(&canonical_target, &body_hash, &canonical_target, &record.body_hash) {
                return Err(StatusCode::CONFLICT);
            }
            
            // Replay detected: Return stored response (simulated by X-Action-Id)
            let mut res = "".into_response(); // Mock empty success
            res.headers_mut().insert("X-Action-Id", record.action_id.parse().unwrap());
            res.headers_mut().insert("X-Idempotency-Replay", "true".parse().unwrap());
            return Ok(res);
        }
    }

    // Process request
    let response = next.run(req).await;

    // Store successful result (Atomic Upsert simulation)
    if response.status().is_success() {
        let action_id = response.headers().get("X-Action-Id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("act-new")
            .to_string();

        let mut write_lock = store.write().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        write_lock.insert(scope_key, IdempotencyRecord {
            body_hash,
            action_id,
        });
    }

    Ok(response)
}

use kernel_core::quota::{QuotaLayer, QuotaViolation};

fn violation_to_response(v: &QuotaViolation) -> Response {
    let mut res = violation_to_status(v).into_response();
    for (name, val) in v.headers() {
        res.headers_mut().insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            val.parse().unwrap()
        );
    }
    res
}

/// REQ-QUOTA-HTTP-CONTRACT: Evaluation Priority (System > Tenant > API)
pub async fn quota_middleware(req: Request<Body>, next: Next) -> Result<Response, Response> {
    // 1. System Hard Limit (Critical Protection)
    if req.headers().contains_key("X-Mock-Quota-System") {
        let violation = QuotaViolation { layer: QuotaLayer::SystemHardLimit, retry_after_s: 100 }; 
        return Err(violation_to_response(&violation));
    }

    // 2. Tenant Budget
    if req.headers().contains_key("X-Mock-Quota-Tenant") {
        let violation = QuotaViolation { layer: QuotaLayer::TenantBudget, retry_after_s: 5 };
        return Err(violation_to_response(&violation));
    }

    // 3. API Rate Limit
    if req.headers().contains_key("X-Mock-Quota-Api") {
        let violation = QuotaViolation { layer: QuotaLayer::ApiRateLimit, retry_after_s: 60 };
        return Err(violation_to_response(&violation));
    }

    Ok(next.run(req).await)
}

fn violation_to_status(v: &QuotaViolation) -> StatusCode {
    match v.layer {
        QuotaLayer::SystemHardLimit => StatusCode::SERVICE_UNAVAILABLE, // 503
        _ => StatusCode::TOO_MANY_REQUESTS, // 429
    }
}
