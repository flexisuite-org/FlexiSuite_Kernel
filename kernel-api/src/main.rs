use kernel_api::build_app;
use kernel_api::middleware::MiddlewareConfig;
use kernel_core::auth::KeyManager;
use sea_orm::{ConnectOptions, Database};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,kernel_api=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    if let Err(msg) = kernel_api::auth::init_auth_config() {
        eprintln!("kernel-api startup error (auth): {msg}");
        std::process::exit(1);
    }

    // Initialize Database
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|e| {
        eprintln!("DATABASE_URL must be set: {e}");
        std::process::exit(1);
    });
    let mut opt = ConnectOptions::new(db_url);
    opt.max_connections(100)
        .min_connections(5)
        .connect_timeout(Duration::from_secs(5))
        .acquire_timeout(Duration::from_secs(30))
        .sqlx_logging(false);

    let db = std::sync::Arc::new(Database::connect(opt).await.unwrap_or_else(|e| {
        eprintln!("Failed to connect to database: {e}");
        std::process::exit(1);
    }));

    use kernel_data::auth_context::{SystemTenantContext, TenantContext};
    let init_ctx = TenantContext::from(SystemTenantContext).with_db(db.clone());
    KeyManager::rotate_keys(&init_ctx)
        .await
        .unwrap_or_else(|e| {
            eprintln!(
                "Failed to initialize key rotation state: {e}. If this is due to missing key-management migrations, run migrations for key_record or perform a preflight migration check."
            );
            std::process::exit(1);
        });

    let config = MiddlewareConfig::default();

    // Start the KID revocation listener (REQ-KEY-REVOCATION-SLO: p95 ≤ 60s).
    // MiddlewareConfig::default() already reads REDIS_URL, so we reuse config.redis_url
    // rather than reading the environment variable a second time.
    let kid_revocation_handle = match redis::Client::open(config.redis_url.clone()) {
        Ok(redis_client) => {
            tracing::info!("Starting KID revocation listener (Redis pub/sub + 30s polling)");
            kernel_api::auth::start_kid_revocation_listener(redis_client)
        }
        Err(e) => {
            tracing::error!(
                "KID revocation listener initialization failed: {}. Aborting startup.",
                e
            );
            std::process::exit(1);
        }
    };
    let (app, _cleanup_handle) = build_app(config, db.clone()).await.unwrap_or_else(|e| {
        eprintln!("kernel-api startup error (middleware): {e}");
        std::process::exit(1);
    });

    let host = std::env::var("KERNEL_API_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("KERNEL_API_PORT").unwrap_or_else(|_| "3000".to_string());
    let addr_str = format!("{host}:{port}");
    let addr: SocketAddr = addr_str.parse().unwrap_or_else(|_| {
        eprintln!("Invalid bind address: {addr_str}");
        std::process::exit(1);
    });
    tracing::info!("Listening on http://{}", addr);

    let listener = TcpListener::bind(addr).await.unwrap_or_else(|e| {
        eprintln!("Failed to bind to {addr}: {e}");
        std::process::exit(1);
    });
    let mut kid_revocation_handle = kid_revocation_handle;
    tokio::select! {
        revocation_result = &mut kid_revocation_handle => {
            match revocation_result {
                Ok(()) => {
                    eprintln!("KID revocation listener exited unexpectedly");
                }
                Err(e) => {
                    eprintln!("KID revocation listener task failed: {e}");
                }
            }
            std::process::exit(1);
        }
        server_result = axum::serve(listener, app) => {
            if let Err(e) = server_result {
                eprintln!("Server error: {e}");
                kid_revocation_handle.abort();
                std::process::exit(1);
            }
            kid_revocation_handle.abort();
        }
    }
}
