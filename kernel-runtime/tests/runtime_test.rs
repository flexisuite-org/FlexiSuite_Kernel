use kernel_runtime::{deno_runtime::DenoSandbox, wasm_runtime::WasmSandbox, SandboxRuntime, RuntimeOptions};
use std::time::Duration;

#[tokio::test]
async fn test_deno_execution() {
    let options = RuntimeOptions::default();
    let mut runtime = DenoSandbox::new(options);
    let code = "const a = 1; a";
    let input = serde_json::Value::Null;
    let result = runtime.execute(code, input).await;
    assert!(result.is_ok(), "Deno execution failed: {:?}", result.err());
    let value = result.unwrap();
    assert_eq!(value, serde_json::json!(1));
}

#[tokio::test]
async fn test_deno_timeout() {
    let mut options = RuntimeOptions::default();
    options.wall_clock_limit = Duration::from_millis(100);
    let mut runtime = DenoSandbox::new(options);
    let code = "while(true) {}";
    let input = serde_json::Value::Null;
    let result = runtime.execute(code, input).await;
    assert!(result.is_err(), "Deno execution should have timed out");
}

#[tokio::test]
async fn test_wasm_execution() {
    let wat = r#"
    (module
        (func $start (export "_start")
            nop
        )
    )
    "#;

    let options = RuntimeOptions::default();
    let mut runtime = WasmSandbox::new(options).unwrap();
    let input = serde_json::Value::Null;
    let result = runtime.execute(wat, input).await;
    assert!(result.is_ok(), "Wasm execution failed: {:?}", result.err());
    // Output check skipped for Wasm as strictly implemented (Null)
    assert_eq!(result.unwrap(), serde_json::Value::Null);
}

#[tokio::test]
async fn test_wasm_timeout() {
    let wat = r#"
    (module
        (func $start (export "_start")
            loop br 0 end
        )
    )
    "#;
    let mut options = RuntimeOptions::default();
    options.wall_clock_limit = Duration::from_millis(100);
    let mut runtime = WasmSandbox::new(options).unwrap();
    let input = serde_json::Value::Null;
    let result = runtime.execute(wat, input).await;
    assert!(result.is_err(), "Wasm execution should have timed out");
}
