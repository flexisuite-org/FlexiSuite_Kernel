pub mod error;
pub mod model;
pub mod storage;
pub mod trust;

/// Spawns a background task that periodically reloads trust root keys.
///
/// This satisfies REQ-KEY-REVOCATION-SLO by ensuring that environment variable
/// changes (e.g. from a config map update) are picked up by the process within 30 seconds.
///
/// Returns a `tokio::task::JoinHandle` that can be awaited or dropped.
pub fn start_trust_root_reloader() -> tokio::task::JoinHandle<()> {
    use std::time::Duration;
    use tracing::{error, info};

    tokio::spawn(async {
        info!("Starting trust root reloader background task");
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        // First tick is immediate
        interval.tick().await;

        loop {
            interval.tick().await;
            // Use spawn_blocking because reload_trust_root_keys acquires a RwLock and does env I/O
            let result = tokio::task::spawn_blocking(move || {
                storage::reload_trust_root_keys();
            })
            .await;

            if let Err(e) = result {
                error!("Trust root reloader task panicked or failed: {}", e);
            }
        }
    })
}
