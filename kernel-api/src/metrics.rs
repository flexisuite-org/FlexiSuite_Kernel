use opentelemetry::{global, metrics::{Meter, Unit, Histogram}, KeyValue};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use std::sync::OnceLock;

pub static METER: OnceLock<Meter> = OnceLock::new();
static INIT: std::sync::Once = std::sync::Once::new();

// Cache instruments
static API_LATENCY: OnceLock<Histogram<f64>> = OnceLock::new();
static SANDBOX_WARM_START: OnceLock<Histogram<f64>> = OnceLock::new();
static SANDBOX_COLD_START: OnceLock<Histogram<f64>> = OnceLock::new();
static SLO_ENV_MATCH: OnceLock<prometheus::Gauge> = OnceLock::new();

pub fn init_metrics() {
    INIT.call_once(|| {
        let registry = prometheus::default_registry();
        let exporter = opentelemetry_prometheus::exporter()
            .with_registry(registry.clone())
            .build()
            .unwrap();

        let provider = SdkMeterProvider::builder()
            .with_reader(exporter)
            .build();

        global::set_meter_provider(provider.clone());

        let meter = global::meter("kernel-api");
        // Clone meter before setting to OnceLock because we need it for instruments
        if METER.set(meter.clone()).is_err() {
            // Already initialized, assume instruments are also initialized or this is redundant check
            return;
        }

        // Initialize and cache histograms
        let api_hist = meter.f64_histogram("api_request_duration")
            .with_description("API request duration in seconds")
            .with_unit(Unit::new("s"))
            .init();
        let _ = API_LATENCY.set(api_hist);

        let warm_hist = meter.f64_histogram("sandbox_warm_start_duration")
            .with_description("Sandbox warm start duration in seconds")
            .with_unit(Unit::new("s"))
            .init();
        let _ = SANDBOX_WARM_START.set(warm_hist);

        let cold_hist = meter.f64_histogram("sandbox_cold_start_duration")
            .with_description("Sandbox cold start duration in seconds")
            .with_unit(Unit::new("s"))
            .init();
        let _ = SANDBOX_COLD_START.set(cold_hist);

        // Register standard availability gauge
        let gauge = prometheus::Gauge::new("availability", "Service availability status").unwrap();
        if registry.register(Box::new(gauge.clone())).is_ok() {
            gauge.set(1.0);
        }

        // Initialize SLO env match gauge
        let slo_gauge = prometheus::Gauge::new("slo_environment_match", "SLO environment profile match status").unwrap();
        if registry.register(Box::new(slo_gauge.clone())).is_ok() {
            let _ = SLO_ENV_MATCH.set(slo_gauge);
        }
    });
}

pub fn get_meter() -> &'static Meter {
    METER.get().expect("Metrics not initialized")
}

pub fn record_api_latency(duration_seconds: f64, method: &str, route: &str, status: u16) {
    if let Some(hist) = API_LATENCY.get() {
        hist.record(
            duration_seconds,
            &[
                KeyValue::new("method", method.to_string()),
                KeyValue::new("route", route.to_string()),
                KeyValue::new("status", status.to_string()),
            ]
        );
    }
}

pub fn set_slo_env_match(matched: bool) {
    if let Some(gauge) = SLO_ENV_MATCH.get() {
        gauge.set(if matched { 1.0 } else { 0.0 });
    } else {
        // If not initialized yet or failed to register (e.g. race condition), we can't set it.
        // But init_metrics is usually called before this.
        tracing::warn!("SLO_ENV_MATCH gauge not initialized when setting value");
    }
}

pub fn record_sandbox_warm_start(duration_seconds: f64) {
    if let Some(hist) = SANDBOX_WARM_START.get() {
        hist.record(duration_seconds, &[]);
    }
}

pub fn record_sandbox_cold_start(duration_seconds: f64) {
    if let Some(hist) = SANDBOX_COLD_START.get() {
        hist.record(duration_seconds, &[]);
    }
}
