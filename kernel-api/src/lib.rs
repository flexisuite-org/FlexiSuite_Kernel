use axum::{
    routing::{get, post}, 
    Router,
    middleware::from_fn,
    Extension,
};
use crate::auth::auth_middleware;
use crate::middleware::{idempotency_middleware, quota_middleware, IdempotencyStore};

pub mod auth;
pub mod middleware;

pub fn build_app(idempotency_store: IdempotencyStore) -> Router {
    Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/test", post(|| async { "OK" }).put(|| async { "OK" }))
        // Register Middlewares (Outermost applied last)
        .layer(from_fn(quota_middleware))
        .layer(from_fn(idempotency_middleware))
        .layer(from_fn(auth_middleware))
        // Provide States
        .layer(Extension(idempotency_store))
}
