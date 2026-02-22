use axum::{
    body::Body,
    http::{Response, StatusCode, header},
    response::IntoResponse,
};
use prometheus::{Encoder, TextEncoder, CounterVec, Histogram, register_counter_vec, register_histogram};
use std::sync::OnceLock;

pub fn init_metrics() {
    // Force initialization of metrics
    let _ = QUOTA_REJECT_TOTAL.get_or_init(|| {
        register_counter_vec!(
            "quota_reject_total",
            "Total number of requests rejected by quota system",
            &["layer"]
        ).unwrap()
    });

    let _ = SANDBOX_DURATION_SECONDS.get_or_init(|| {
        register_histogram!(
            "sandbox_duration_seconds",
            "Duration of sandbox execution in seconds",
            vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0]
        ).unwrap()
    });

    let _ = TOKEN_VALIDATION_TOTAL.get_or_init(|| {
        register_counter_vec!(
            "token_validation_total",
            "Total number of token validations",
            &["status"]
        ).unwrap()
    });
}

pub async fn metrics_handler() -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
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

static QUOTA_REJECT_TOTAL: OnceLock<CounterVec> = OnceLock::new();
static SANDBOX_DURATION_SECONDS: OnceLock<Histogram> = OnceLock::new();
static TOKEN_VALIDATION_TOTAL: OnceLock<CounterVec> = OnceLock::new();

pub fn record_quota_reject(layer: &str) {
    let counter = QUOTA_REJECT_TOTAL
        .get()
        .expect("metrics not initialized — call init_metrics() at startup");
    counter.with_label_values(&[layer]).inc();
}

pub fn record_sandbox_duration(duration_seconds: f64) {
    let histogram = SANDBOX_DURATION_SECONDS
        .get()
        .expect("metrics not initialized — call init_metrics() at startup");
    histogram.observe(duration_seconds);
}

pub fn record_token_validation(status: &str) {
    let counter = TOKEN_VALIDATION_TOTAL
        .get()
        .expect("metrics not initialized — call init_metrics() at startup");
    counter.with_label_values(&[status]).inc();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_initialization_and_recording() {
        init_metrics();

        record_quota_reject("test_layer");
        record_sandbox_duration(0.5);
        record_token_validation("success");

        let response = metrics_handler().await.into_response();
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
