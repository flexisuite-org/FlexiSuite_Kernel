use axum::{
    body::Body,
    http::{Response, StatusCode, header},
    response::IntoResponse,
};
use prometheus::{Encoder, TextEncoder, CounterVec, Histogram, Registry, register_counter_vec_with_registry, register_histogram_with_registry};
use std::sync::OnceLock;

pub fn init_metrics() {
    init_metrics_with_registry(prometheus::default_registry());
}

pub fn init_metrics_with_registry(registry: &Registry) {
    let _ = METRICS.get_or_init(|| Metrics::new(registry));
}

struct Metrics {
    quota_reject_total: CounterVec,
    sandbox_duration_seconds: Histogram,
    token_validation_total: CounterVec,
}

impl Metrics {
    fn new(registry: &Registry) -> Self {
        let quota_reject_total = register_counter_vec_with_registry!(
            "quota_reject_total",
            "Total number of requests rejected by quota system",
            &["layer"],
            registry
        ).unwrap();

        let sandbox_duration_seconds = register_histogram_with_registry!(
            "sandbox_duration_seconds",
            "Duration of sandbox execution in seconds",
            vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0],
            registry
        ).unwrap();

        let token_validation_total = register_counter_vec_with_registry!(
            "token_validation_total",
            "Total number of token validations",
            &["status"],
            registry
        ).unwrap();

        Self {
            quota_reject_total,
            sandbox_duration_seconds,
            token_validation_total,
        }
    }
}

pub async fn metrics_handler() -> impl IntoResponse {
    metrics_handler_with_registry(prometheus::default_registry()).await
}

async fn metrics_handler_with_registry(registry: &Registry) -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let metric_families = registry.gather();
    let mut buffer = vec![];

    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        tracing::error!(error = %e, "Failed to encode Prometheus metrics");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain")],
            format!("Failed to encode metrics: {}", e),
        ).into_response();
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, encoder.format_type())
        .body(Body::from(buffer))
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "Failed to build Prometheus response");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/plain")],
                format!("Failed to build response: {}", e),
            ).into_response()
        })
}

static METRICS: OnceLock<Metrics> = OnceLock::new();

pub fn record_quota_reject(layer: &str) {
    let metrics = METRICS
        .get()
        .expect("metrics not initialized — call init_metrics() at startup");
    metrics.quota_reject_total.with_label_values(&[layer]).inc();
}

pub fn record_sandbox_duration(duration_seconds: f64) {
    let metrics = METRICS
        .get()
        .expect("metrics not initialized — call init_metrics() at startup");
    metrics.sandbox_duration_seconds.observe(duration_seconds);
}

pub fn record_token_validation(status: &str) {
    let metrics = METRICS
        .get()
        .expect("metrics not initialized — call init_metrics() at startup");
    metrics.token_validation_total.with_label_values(&[status]).inc();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_initialization_and_recording() {
        let registry = Registry::new();
        // Exercise the init_metrics_with_registry path which sets the static METRICS
        init_metrics_with_registry(&registry);

        // Record using the global record_* functions which use the static METRICS
        record_quota_reject("test_layer");
        record_sandbox_duration(0.5);
        record_token_validation("success");

        let response = metrics_handler_with_registry(&registry).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body();
        let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
        let output = String::from_utf8(bytes.to_vec()).unwrap();

        assert!(output.contains("quota_reject_total"));
        assert!(output.contains("sandbox_duration_seconds"));
        assert!(output.contains("token_validation_total"));
        assert!(output.contains("layer=\"test_layer\""));
    }
}
