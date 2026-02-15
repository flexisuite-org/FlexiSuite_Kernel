use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode},
    middleware::Next,
    response::{Response, IntoResponse},
};
use crate::auth::TenantContext;
use kernel_core::idempotency::{canonicalize_request_target, check_idempotency_conflict};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

// REQ-IDEMPOTENCY-STORE: Simulation of 24h storage
#[derive(Clone, Debug)]
pub struct IdempotencyRecord {
    pub body_hash: String,
    pub action_id: String,
    pub status: StatusCode,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    // ttl etc omitted for simulation
}

#[derive(Clone, Debug)]
pub enum IdempotencyEntry {
    InFlight {
        body_hash: String,
        notify: Arc<Notify>,
    },
    Completed(IdempotencyRecord),
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct IdempotencyScopeKey {
    tenant_id: String,
    method: String,
    canonical_target: String,
    idempotency_key: String,
}

pub type IdempotencyStore = Arc<Mutex<HashMap<IdempotencyScopeKey, IdempotencyEntry>>>;

/// REQ-IDEMPOTENCY-HEADER: Middleware logic
pub async fn idempotency_middleware(req: Request<Body>, next: Next) -> Result<Response, StatusCode> {
    let (parts, body) = req.into_parts();
    let method = parts.method.clone();

    // Only apply to write methods
    if method == Method::GET || method == Method::HEAD || method == Method::OPTIONS {
        return Ok(next.run(Request::from_parts(parts, body)).await);
    }

    let idempotency_key = match parts.headers.get("Idempotency-Key") {
        Some(val) => val.to_str().map_err(|_| StatusCode::BAD_REQUEST)?.to_string(),
        None => return Ok(next.run(Request::from_parts(parts, body)).await), // Optional header
    };

    let tenant_ctx = parts
        .extensions
        .get::<TenantContext>()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    
    // Canonicalize
    let path = parts.uri.path();
    let query = parts.uri.query();
    let canonical_target = canonicalize_request_target(path, query);

    // Uniqueness Scope: (tenant_id, method, canonical_target, key)
    let scope_key = IdempotencyScopeKey {
        tenant_id: tenant_ctx.tenant_id.clone(),
        method: method.as_str().to_string(),
        canonical_target: canonical_target.clone(),
        idempotency_key,
    };

    // Body hash MUST be derived from the actual request body.
    let body_bytes = to_bytes(body, usize::MAX)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let body_hash = compute_body_hash(&body_bytes);
    let req = Request::from_parts(parts, Body::from(body_bytes));

    // Check Store
    let store = req.extensions().get::<IdempotencyStore>()
        .cloned() // Clone Arc to release borrow of req
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    
    loop {
        let mut lock = store.lock().await;
        let waiter = if let Some(existing) = lock.get(&scope_key) {
            match existing {
                IdempotencyEntry::Completed(record) => {
                    // REQ-IDEMPOTENCY-CONFLICT: Conflict Guard logic
                    if check_idempotency_conflict(&canonical_target, &body_hash, &canonical_target, &record.body_hash) {
                        return Err(StatusCode::CONFLICT);
                    }
                    return Ok(build_replay_response(record));
                }
                IdempotencyEntry::InFlight {
                    body_hash: existing_hash,
                    notify,
                } => {
                    if check_idempotency_conflict(&canonical_target, &body_hash, &canonical_target, existing_hash) {
                        return Err(StatusCode::CONFLICT);
                    }
                    notify.clone()
                }
            }
        } else {
            lock.insert(
                scope_key.clone(),
                IdempotencyEntry::InFlight {
                    body_hash: body_hash.clone(),
                    notify: Arc::new(Notify::new()),
                },
            );
            break;
        };
        drop(lock);
        waiter.notified().await;
    }

    // Process request
    let response = next.run(req).await;

    // Store successful result (Atomic Upsert simulation)
    if response.status().is_success() {
        let (parts, body) = response.into_parts();
        let body_bytes = match to_bytes(body, usize::MAX).await {
            Ok(bytes) => bytes,
            Err(_) => {
                release_inflight(&store, &scope_key).await;
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        };
        let action_id = parts.headers.get("X-Action-Id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("act-new")
            .to_string();
        let stored = IdempotencyRecord {
            body_hash,
            action_id,
            status: parts.status,
            headers: snapshot_headers(&parts.headers),
            body: body_bytes.to_vec(),
        };
        {
            let mut write_lock = store.lock().await;
            let notify = match write_lock.remove(&scope_key) {
                Some(IdempotencyEntry::InFlight { notify, .. }) => notify,
                _ => Arc::new(Notify::new()),
            };
            write_lock.insert(scope_key, IdempotencyEntry::Completed(stored));
            notify.notify_waiters();
        }
        return Ok(Response::from_parts(parts, Body::from(body_bytes)));
    }
    release_inflight(&store, &scope_key).await;

    Ok(response)
}

fn compute_body_hash(body: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    body.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

async fn release_inflight(store: &IdempotencyStore, scope_key: &IdempotencyScopeKey) {
    let mut write_lock = store.lock().await;
    if let Some(IdempotencyEntry::InFlight { notify, .. }) = write_lock.remove(scope_key) {
        notify.notify_waiters();
    }
}

fn snapshot_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_string(), v.to_string()))
        })
        .collect()
}

fn build_replay_response(record: &IdempotencyRecord) -> Response {
    let mut res = Response::builder()
        .status(record.status)
        .body(Body::from(record.body.clone()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());

    for (name, val) in &record.headers {
        if let (Ok(header_name), Ok(header_value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(val),
        ) {
            res.headers_mut().insert(header_name, header_value);
        }
    }
    if let Ok(value) = HeaderValue::from_str("true") {
        res.headers_mut().insert("X-Idempotency-Replay", value);
    }
    res
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
