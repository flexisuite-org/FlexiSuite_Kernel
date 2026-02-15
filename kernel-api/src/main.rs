use kernel_api::build_app;
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let store = std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new()));
    let app = build_app(store);
    
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Listening on http://{}", addr);

    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
