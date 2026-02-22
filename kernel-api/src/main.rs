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
    let (app, metrics_app, _cleanup_handle) =
        build_app(config, db.clone()).await.unwrap_or_else(|e| {
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

    let metrics_host =
        std::env::var("KERNEL_API_METRICS_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let metrics_port =
        std::env::var("KERNEL_API_METRICS_PORT").unwrap_or_else(|_| "9091".to_string());
    let metrics_addr_str = format!("{metrics_host}:{metrics_port}");
    let metrics_addr: SocketAddr = metrics_addr_str.parse().unwrap_or_else(|_| {
        eprintln!("Invalid metrics bind address: {metrics_addr_str}");
        std::process::exit(1);
    });

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let metrics_listener = TcpListener::bind(metrics_addr).await.unwrap_or_else(|e| {
        eprintln!("Failed to bind metrics to {metrics_addr}: {e}");
        std::process::exit(1);
    });
    tracing::info!("Listening on http://{} (Metrics)", metrics_addr);

    tokio::spawn(async move {
        let server = axum::serve(metrics_listener, metrics_app);
        if let Err(e) = server
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
        {
            tracing::error!("Metrics server error: {e}");
        }
    });

    let listener = TcpListener::bind(addr).await.unwrap_or_else(|e| {
        eprintln!("Failed to bind to {addr}: {e}");
        std::process::exit(1);
    });
    tracing::info!("Listening on http://{} (API)", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let ctrl_c = async {
                tokio::signal::ctrl_c()
                    .await
                    .map_err(|e| format!("failed to install CTRL+C handler: {e}"))
            };

            #[cfg(unix)]
            let terminate = async {
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .map_err(|e| format!("failed to install SIGTERM handler: {e}"))?
                    .recv()
                    .await;
                Ok::<(), String>(())
            };

            #[cfg(not(unix))]
            let terminate = std::future::pending::<Result<(), String>>();

            tokio::select! {
                res = ctrl_c => {
                    if let Err(e) = res {
                        tracing::error!("{e}");
                    } else {
                        tracing::info!("Shutdown signal (SIGINT) received, stopping API server");
                    }
                },
                res = terminate => {
                    if let Err(e) = res {
                        tracing::error!("{e}");
                    } else {
                        tracing::info!("Shutdown signal (SIGTERM) received, stopping API server");
                    }
                },
            }
        })
        .await
        .unwrap_or_else(|e| {
            eprintln!("Server error: {e}");
            std::process::exit(1);
        });

    let _ = shutdown_tx.send(());
}
