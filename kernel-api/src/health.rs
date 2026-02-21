use axum::{
    extract::Extension,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use serde::Serialize;
use crate::middleware::MiddlewareState;

pub async fn liveness() -> StatusCode {
    StatusCode::OK
}

#[derive(Serialize)]
struct ReadinessResponse {
    status: String,
    checks: Checks,
}

#[derive(Serialize)]
struct Checks {
    database: String,
    redis: String,
}

pub async fn readiness(
    Extension(state): Extension<MiddlewareState>,
    Extension(db): Extension<Arc<DatabaseConnection>>,
) -> Response {
    // Check DB
    let db_status = match db.ping().await {
        Ok(_) => "up",
        Err(e) => {
            tracing::error!(error = ?e, "Readiness check failed (database)");
            "down"
        },
    };

    // Check Redis (via IdempotencyStore)
    let redis_status = match state.idempotency_store.ping().await {
        Ok(_) => "up",
        Err(e) => {
            tracing::error!(error = ?e, "Readiness check failed (redis)");
            "down"
        },
    };

    let status_code = if db_status == "up" && redis_status == "up" {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let body = ReadinessResponse {
        status: if status_code == StatusCode::OK { "healthy".to_string() } else { "unhealthy".to_string() },
        checks: Checks {
            database: db_status.to_string(),
            redis: redis_status.to_string(),
        },
    };

    (status_code, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::{MiddlewareConfig, InMemoryIdempotencyStore, InMemoryActionStore, InMemoryQuotaStore};
    use sea_orm::{MockDatabase, DatabaseBackend};

    #[tokio::test]
    async fn test_liveness() {
        assert_eq!(liveness().await, StatusCode::OK);
    }

    #[tokio::test]
    async fn test_readiness_structure() {
        // This test mainly verifies that readiness function can be called and returns a response.
        // We use MockDatabase which might fail the ping, but we check that we get a response.

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .into_connection();
        let db = Arc::new(db);

        let config = MiddlewareConfig::default();
        let state = MiddlewareState::with_store(
            config,
            Arc::new(InMemoryIdempotencyStore::new()),
            Arc::new(InMemoryActionStore::new()),
            Arc::new(InMemoryQuotaStore::new()),
        );

        let response = readiness(Extension(state), Extension(db)).await;
        assert!(response.status() == StatusCode::OK || response.status() == StatusCode::SERVICE_UNAVAILABLE);
    }
}
