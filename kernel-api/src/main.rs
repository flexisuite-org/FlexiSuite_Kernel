use kernel_api::build_app;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

#[tokio::main]
async fn main() {
    if let Err(msg) = validate_required_env() {
        eprintln!("kernel-api startup error: {msg}");
        std::process::exit(1);
    }

    let app = build_app();

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Listening on http://{}", addr);

    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

fn validate_required_env() -> Result<(), String> {
    let key = std::env::var("FLEXI_PASETO_V4_PUBLIC_KEY_B64URL")
        .map_err(|_| "FLEXI_PASETO_V4_PUBLIC_KEY_B64URL is not set".to_string())?;
    let decoded = URL_SAFE_NO_PAD
        .decode(key)
        .map_err(|_| "FLEXI_PASETO_V4_PUBLIC_KEY_B64URL must be base64url (no padding)".to_string())?;
    if decoded.len() != 32 {
        return Err("FLEXI_PASETO_V4_PUBLIC_KEY_B64URL must decode to 32-byte Ed25519 public key".to_string());
    }
    Ok(())
}
