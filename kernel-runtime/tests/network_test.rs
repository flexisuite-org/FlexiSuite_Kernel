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
                            let mut buf = [0; 1024];
                            let _ = socket.read(&mut buf).await;
                            let response = "HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\nHello Network";
                            let _ = socket.write_all(response.as_bytes()).await;
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
    // Allowlist expects host (e.g. 127.0.0.1). But url has port.
    // check_url uses Url::host_str().
    // If I pass "127.0.0.1:port" to allowlist, check_url check:
    // url.host_str() returns "127.0.0.1".
    // So allowlist should have "127.0.0.1".
    // Wait, my check_url implementation: `if !allowlist.iter().any(|allowed| allowed == host)`
    // host is from url.host_str().
    // So allowlist must contain hostname without port.
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

    // Use a valid URL that is NOT in the allowlist
    // allowlist has "google.com".
    // We try to fetch "http://example.com".
    // Or we use `start_mock_server` url (127.0.0.1) but allowlist has google.com.
    // Yes, that's better.
    let _target_url = "http://127.0.0.1:12345"; // Port doesn't matter for allowlist check usually, but we need valid URL.
    // Wait, if I use start_mock_server url, it is 127.0.0.1.
    // Allowlist has google.com.
    // So fetch(127.0.0.1) should fail.

    // But previous test code used `url` variable which is 127.0.0.1.
    // So why did it fail with invalid argument?
    // Ah, I see: `options.permissions.network_allowlist = vec!["google.com".to_string()];`
    // `let code = format!(..., url)`.
    // `url` IS `http://127.0.0.1:...`.
    // So `Url::parse` SHOULD succeed.
    // So why `invalid_argument`?
    // Maybe `op_fetch` args?
    // `op_fetch` takes `url_str: String`.
    // Maybe `check_url`?
    // `crate::check_url` uses `Url::parse`.
    // If `url` is `http://127.0.0.1:xxxx`, it parses fine.
    // `host_str` is `127.0.0.1`.
    // Allowlist has `google.com`.
    // `any` returns false.
    // Returns Err("Network access to '127.0.0.1' is not allowed").
    // Mapped to `JsErrorBox::new("PermissionDenied", msg)`.
    // So JS error should have that message.
    // But test got "invalid_argument".
    // This implies `JsErrorBox::new("PermissionDenied", msg)` creates an error with message "invalid_argument"?
    // OR "PermissionDenied" class maps to `invalid_argument` code/message in Deno?
    // "PermissionDenied" is standard Deno error.
    // Maybe `deno_error` 0.7.3 maps it differently?
    // Or maybe I should use `JsErrorBox::generic` for everything to preserve message?
    // Or `JsErrorBox::new("Error", msg)`.
    // I'll try `JsErrorBox::generic` in `deno_runtime.rs` for PermissionDenied case too, to be safe about message preservation.
    // But wait, I need to distinguish types?
    // For this test, I check message.
    // I'll assume "PermissionDenied" class is doing something weird with message.

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

    assert!(
        result.is_err(),
        "Expected Wasm execution to fail (trap) on denied network"
    );
    // We accept any runtime error (trap)
}
