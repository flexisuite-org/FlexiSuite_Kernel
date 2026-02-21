use kernel_runtime::{
    RuntimeOptions, SandboxRuntime, deno_runtime::DenoSandbox, wasm_runtime::WasmSandbox,
};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;

async fn start_mock_server() -> (String, Arc<Notify>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{}", addr);
    let notify = Arc::new(Notify::new());
    let notify_clone = notify.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                res = listener.accept() => {
                    match res {
                        Ok((mut socket, _)) => {
                            tokio::spawn(async move {
                                let mut buf = [0; 1024];
                                let mut request_data = Vec::new();
                                loop {
                                    let n = match socket.read(&mut buf).await {
                                        Ok(n) if n == 0 => return, // Connection closed
                                        Ok(n) => n,
                                        Err(_) => return,
                                    };
                                    request_data.extend_from_slice(&buf[..n]);
                                    if request_data.windows(4).any(|window| window == b"\r\n\r\n") {
                                        break;
                                    }
                                }
                                let response = "HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nHello Network";
                                let _ = socket.write_all(response.as_bytes()).await;
                                let _ = socket.flush().await;
                                let _ = socket.shutdown().await;
                            });
                        }
                        Err(_) => break,
                    }
                }
                _ = notify_clone.notified() => {
                    break;
                }
            }
        }
    });

    (url, notify)
}

#[tokio::test]
async fn test_deno_network_allowed() {
    let (url, stop_server) = start_mock_server().await;
    let _host = url.replace("http://", ""); // 127.0.0.1:port
    let hostname = "127.0.0.1";

    let mut options = RuntimeOptions::default();
    options.permissions.network_allowlist = vec![hostname.to_string()];
    let mut runtime = DenoSandbox::new(options);

    let code = format!(
        r#"
        (async () => {{
            const res = await fetch("{}", {{}});
            const text = await res.text();
            return text;
        }})()
        "#,
        url
    );

    let result = runtime.execute(&code, serde_json::Value::Null).await;
    stop_server.notify_one();

    assert!(result.is_ok(), "Execution failed: {:?}", result.err());
    assert_eq!(result.unwrap(), serde_json::json!("Hello Network"));
}

#[tokio::test]
async fn test_deno_network_denied() {
    let (url, stop_server) = start_mock_server().await;
    // Don't add to allowlist
    let mut options = RuntimeOptions::default();
    options.permissions.network_allowlist = vec!["google.com".to_string()];
    let mut runtime = DenoSandbox::new(options);

    let code = format!(
        r#"
        (async () => {{
            try {{
                await fetch("{}", {{}});
                return "Allowed";
            }} catch (e) {{
                return e.message; // maybe return e.name + ": " + e.message
            }}
        }})()
        "#,
        url
    );

    let result = runtime.execute(&code, serde_json::Value::Null).await;
    stop_server.notify_one();

    let output = result.unwrap();
    let output_str = output.as_str().unwrap();
    // Verify it caught the error
    assert!(
        output_str.contains("Network access to"),
        "Expected network error, got: {}",
        output_str
    );
    assert!(
        output_str.contains("not allowed"),
        "Expected 'not allowed', got: {}",
        output_str
    );
}

#[tokio::test]
async fn test_wasm_network_allowed() {
    let (url, stop_server) = start_mock_server().await;
    let hostname = "127.0.0.1";

    let mut options = RuntimeOptions::default();
    options.permissions.network_allowlist = vec![hostname.to_string()];
    let mut runtime = WasmSandbox::new(options).unwrap();

    // Wasm module that calls flexi_fetch
    // (import "env" "flexi_fetch" (func $fetch (param i32 i32 i32 i32) (result i32)))
    // memory ...
    // data ... url
    // call fetch

    let wat = format!(
        r#"
        (module
            (import "env" "flexi_fetch" (func $fetch (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "{}") ;; URL
            (data (i32.const 1000) "GET")
            (data (i32.const 1010) "{{}}")
            (data (i32.const 1020) "")

            (func (export "_start")
                (call $fetch
                    (i32.const 0) (i32.const {}) ;; url
                    (i32.const 1000) (i32.const 3) ;; method
                    (i32.const 1010) (i32.const 2) ;; headers
                    (i32.const 1020) (i32.const 0) ;; body
                    (i32.const 2000) (i32.const 1000) ;; out
                )
                drop
            )
        )
        "#,
        url,
        url.len()
    );

    let result = runtime.execute(&wat, serde_json::Value::Null).await;
    stop_server.notify_one();

    assert!(result.is_ok(), "Wasm execution failed: {:?}", result.err());
}

#[tokio::test]
async fn test_wasm_network_denied() {
    let (url, stop_server) = start_mock_server().await;

    let mut options = RuntimeOptions::default();
    options.permissions.network_allowlist = vec!["google.com".to_string()];
    let mut runtime = WasmSandbox::new(options).unwrap();

    let wat = format!(
        r#"
        (module
            (import "env" "flexi_fetch" (func $fetch (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "{}")
            (data (i32.const 1000) "GET")
            (data (i32.const 1010) "{{}}")
            (data (i32.const 1020) "")

            (func (export "_start")
                (local i32)
                (call $fetch
                    (i32.const 0) (i32.const {}) ;; url
                    (i32.const 1000) (i32.const 3)
                    (i32.const 1010) (i32.const 2)
                    (i32.const 1020) (i32.const 0)
                    (i32.const 2000) (i32.const 1000)
                )

                (local.set 0) ;; save result
                (if (i32.lt_s (local.get 0) (i32.const 0))
                    (then
                        unreachable ;; Trap if negative (error)
                    )
                )
            )
            (func (export "check") (param i32) (result i32)
                local.get 0
            )
        )
        "#,
        url,
        url.len()
    );

    let result = runtime.execute(&wat, serde_json::Value::Null).await;
    stop_server.notify_one();

    // We expect it to FAIL (RuntimeError or Trap) because of unreachable
    assert!(
        result.is_err(),
        "Expected Wasm execution to fail (trap) on denied network"
    );
    // We accept any runtime error (trap)
}
