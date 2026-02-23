use axum::{
    body::Body,
    http::{Response, StatusCode, header},
    response::IntoResponse,
};
use prometheus::{Encoder, TextEncoder, CounterVec, Histogram, Registry, register_counter_vec_with_registry, register_histogram_with_registry};
use std::sync::OnceLock;

pub fn init_metrics() {
    let _ = init_metrics_with_registry(prometheus::default_registry());
}

pub fn init_metrics_with_registry(registry: &Registry) -> Result<(), prometheus::Error> {
    // Check if already initialized before attempting to register metrics in the registry.
    // This avoids unnecessary registration and focuses registry errors on actual duplicates.
    if METRICS.get().is_some() {
        return Err(prometheus::Error::Msg(
            "Metrics already initialized".to_string(),
        ));
    }

    let metrics = Metrics::new(registry)?;
    if let Err(metrics) = METRICS.set(metrics) {
        // Already initialized (handled race where another thread set it after the check above).
        // To avoid leaving orphaned metrics in the passed registry, we attempt to unregister them.
        let _ = registry.unregister(Box::new(metrics.quota_reject_total));
        let _ = registry.unregister(Box::new(metrics.sandbox_duration_seconds));
        let _ = registry.unregister(Box::new(metrics.token_validation_total));

        return Err(prometheus::Error::Msg(
            "Metrics already initialized".to_string(),
        ));
    }
    Ok(())
}

struct Metrics {
    quota_reject_total: CounterVec,
    sandbox_duration_seconds: Histogram,
    token_validation_total: CounterVec,
}

impl Metrics {
    fn new(registry: &Registry) -> Result<Self, prometheus::Error> {
        let quota_reject_total = register_counter_vec_with_registry!(
            "quota_reject_total",
            "Total number of requests rejected by quota system",
            &["layer"],
            registry
        )?;

        let sandbox_duration_seconds = register_histogram_with_registry!(
            "sandbox_duration_seconds",
            "Duration of sandbox execution in seconds",
            vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0],
            registry
        )?;

        let token_validation_total = register_counter_vec_with_registry!(
            "token_validation_total",
            "Total number of token validations",
            &["status"],
            registry
        )?;

        Ok(Self {
            quota_reject_total,
            sandbox_duration_seconds,
            token_validation_total,
        })
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

use std::sync::atomic::{AtomicBool, Ordering};
static METRICS: OnceLock<Metrics> = OnceLock::new();
static WARNED_UNINITIALIZED: AtomicBool = AtomicBool::new(false);

fn warn_uninitialized() {
    if !WARNED_UNINITIALIZED.swap(true, Ordering::Relaxed) {
        tracing::warn!("Metrics not initialized — recording calls will be no-ops. Call init_metrics() at startup.");
    }
}

pub fn record_quota_reject(layer: &str) {
    if let Some(metrics) = METRICS.get() {
        metrics.quota_reject_total.with_label_values(&[layer]).inc();
    } else {
        warn_uninitialized();
    }
}

pub fn record_sandbox_duration(duration_seconds: f64) {
    if let Some(metrics) = METRICS.get() {
        metrics.sandbox_duration_seconds.observe(duration_seconds);
    } else {
        warn_uninitialized();
    }
}

pub fn record_token_validation(status: &str) {
    if let Some(metrics) = METRICS.get() {
        metrics.token_validation_total.with_label_values(&[status]).inc();
    } else {
        warn_uninitialized();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_metrics_initialization_and_recording() {
        let registry = Registry::new();
        // Since METRICS is a OnceLock, we can only initialize it once per process.
        // For testing, we use a local Metrics instance with its own registry to avoid races.
        let metrics = Metrics::new(&registry).unwrap();

        metrics.quota_reject_total.with_label_values(&["test_layer"]).inc();
        metrics.sandbox_duration_seconds.observe(0.5);
        metrics.token_validation_total.with_label_values(&["success"]).inc();

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

    #[tokio::test]
    async fn test_multiple_initialization_fails() {
        // Since METRICS is a global OnceLock, these tests may interact if run in parallel.
        // Rust tests run in parallel by default, but individual tests here attempt to
        // register with different registries. However, METRICS.set() will still fail
        // if any other test already initialized it.
        let registry1 = Registry::new();
        let _ = init_metrics_with_registry(&registry1);
        
        // Second initialization must fail even with a fresh Registry.
        // This ensures we are testing the OnceLock (METRICS.set()) failure path,
        // not a duplicate registration error within the same Registry.
        let registry2 = Registry::new();
        let res2 = init_metrics_with_registry(&registry2);
        assert!(res2.is_err());
        if let Err(prometheus::Error::Msg(msg)) = res2 {
            assert_eq!(msg, "Metrics already initialized");
        }
    }
}
