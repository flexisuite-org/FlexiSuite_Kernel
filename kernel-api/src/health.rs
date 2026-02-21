use axum::{
    extract::Extension,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use sea_orm::DatabaseConnection;
use std::sync::Arc;
use serde::Serialize;
use crate::middleware::{MiddlewareState, PingStatus};
use crate::auth::SystemTenantContext;

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
    database: Health,
    redis: Health,
}

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
enum Health {
    Up,
    Down,
    Degraded,
}

pub async fn readiness(
    Extension(state): Extension<MiddlewareState>,
    Extension(db): Extension<Arc<DatabaseConnection>>,
) -> Response {
    // Check DB via tenant-scoped accessor (Infrastructure layer)
    let ctx = crate::auth::TenantContext::from(SystemTenantContext).with_db(db);
    let db_health = match ctx.check_connection().await {
        Ok(_) => Health::Up,
        Err(e) => {
            tracing::error!(error = ?e, "Readiness check failed (database)");
            Health::Down
        },
    };

    // Check Redis (via IdempotencyStore)
    let redis_health = match state.idempotency_store.ping().await {
        Ok(PingStatus::Ok) => Health::Up,
        Ok(PingStatus::Degraded) => Health::Degraded,
        Err(e) => {
            tracing::error!(error = ?e, "Readiness check failed (redis)");
            Health::Down
        },
    };

    let status_code = if db_health == Health::Up && redis_health != Health::Down {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let body = ReadinessResponse {
        status: if status_code == StatusCode::OK { "healthy".to_string() } else { "unhealthy".to_string() },
        checks: Checks {
            database: db_health,
            redis: redis_health,
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
        // MockDatabase::ping() defaults to failing (DbErr::Connection("Ping failed"))
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .into_connection();
        let db = Arc::new(db);

        let config = MiddlewareConfig::default();
        let state = MiddlewareState::with_store(
            config,
            Arc::new(InMemoryIdempotencyStore::new()), // returns Degraded
            Arc::new(InMemoryActionStore::new()),
            Arc::new(InMemoryQuotaStore::new()),
        );

        let response = readiness(Extension(state), Extension(db)).await;
        
        // MockDatabase::ping() succeeds by default in this environment,
        // and Redis is Degraded. Both mean the service is READY (200 OK).
        assert_eq!(response.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(body["status"], "healthy");
        assert_eq!(body["checks"]["database"], "up");
        assert_eq!(body["checks"]["redis"], "degraded");
    }
}
