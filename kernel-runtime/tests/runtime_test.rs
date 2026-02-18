use kernel_runtime::{
    RuntimeOptions, SandboxError, SandboxRuntime, deno_runtime::DenoSandbox,
    wasm_runtime::WasmSandbox,
};
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
    match runtime.execute(code, input).await {
        Err(SandboxError::Timeout) => {}
        other => panic!("Expected Timeout, got: {:?}", other),
    }
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
    assert_eq!(result.unwrap(), serde_json::json!(""));
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
    match runtime.execute(wat, input).await {
        Err(SandboxError::Timeout) => {}
        other => panic!("Expected Timeout, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_deno_memory_limit() {
    let mut options = RuntimeOptions::default();
    options.memory_limit = 8 * 1024 * 1024;
    options.wall_clock_limit = Duration::from_secs(2);
    let mut runtime = DenoSandbox::new(options);
    let code = "const x = new Uint8Array(128 * 1024 * 1024); x.length;";
    let input = serde_json::Value::Null;
    match runtime.execute(code, input).await {
        Err(SandboxError::MemoryLimitExceeded) => {}
        other => panic!("Expected MemoryLimitExceeded, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_deno_cpu_limit() {
    let mut options = RuntimeOptions::default();
    options.cpu_time_limit = Duration::from_millis(50);
    options.wall_clock_limit = Duration::from_secs(2);
    let mut runtime = DenoSandbox::new(options);
    let code = "while (true) {}";
    let input = serde_json::Value::Null;
    match runtime.execute(code, input).await {
        Err(SandboxError::CpuLimitExceeded) => {}
        other => panic!("Expected CpuLimitExceeded, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_wasm_memory_limit() {
    let wat = r#"
    (module
        (memory 1024)
        (func (export "_start")
            nop
        )
    )
    "#;
    let mut options = RuntimeOptions::default();
    options.memory_limit = 1024 * 1024;
    let mut runtime = WasmSandbox::new(options).unwrap();
    let input = serde_json::Value::Null;
    assert!(runtime.execute(wat, input).await.is_err());
}

#[tokio::test]
async fn test_wasm_invalid_wat() {
    let wat = "(module (func";
    let options = RuntimeOptions::default();
    let mut runtime = WasmSandbox::new(options).unwrap();
    let input = serde_json::Value::Null;
    assert!(runtime.execute(wat, input).await.is_err());
}

#[tokio::test]
async fn test_wasm_missing_start_export() {
    let wat = r#"
    (module
        (func (export "run")
            nop
        )
    )
    "#;
    let options = RuntimeOptions::default();
    let mut runtime = WasmSandbox::new(options).unwrap();
    let input = serde_json::Value::Null;
    assert!(runtime.execute(wat, input).await.is_err());
}
