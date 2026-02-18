use kernel_api::build_app;
use kernel_api::middleware::MiddlewareConfig;
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

    if let Err(msg) = kernel_data::init_hmac_secret() {
        eprintln!("kernel-api startup error (data): {msg}");
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

    let db = Database::connect(opt).await.unwrap_or_else(|e| {
        eprintln!("Failed to connect to database: {e}");
        std::process::exit(1);
    });

    let config = MiddlewareConfig::default();
    let (app, _cleanup_handle) = build_app(config, db).await;

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
    axum::serve(listener, app).await.unwrap_or_else(|e| {
        eprintln!("Server error: {e}");
        std::process::exit(1);
    });
}
