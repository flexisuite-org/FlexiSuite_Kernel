use kernel_api::build_app;
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    if let Err(msg) = kernel_api::auth::init_auth_config() {
        eprintln!("kernel-api startup error: {msg}");
        std::process::exit(1);
    }

    let app = build_app();

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Listening on http://{}", addr);

    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
