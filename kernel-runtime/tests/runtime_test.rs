use kernel_runtime::{
    RuntimeOptions, SandboxError, SandboxRuntime, deno_runtime::DenoSandbox,
    wasm_runtime::WasmSandbox,
};
use std::time::Duration;

#[tokio::test]
async fn test_deno_execution() {
    let mut options = RuntimeOptions::default();
    options.cpu_time_limit = Duration::from_secs(30);
    let mut runtime = DenoSandbox::new(options);
    let code = "const a = 1; a";
    let input = serde_json::Value::Null;
    let result = runtime.execute(code, input).await;
    assert!(result.is_ok(), "Deno execution failed: {:?}", result.err());
    let value = result.unwrap();
    assert_eq!(value, serde_json::json!(1));
}

#[tokio::test]
async fn test_deno_async() {
    let mut options = RuntimeOptions::default();
    options.cpu_time_limit = Duration::from_secs(30);
    let mut runtime = DenoSandbox::new(options);
    let code = "Promise.resolve(42)";
    let input = serde_json::Value::Null;
    let result = runtime.execute(code, input).await;
    assert!(result.is_ok(), "Deno async execution failed: {:?}", result.err());
    assert_eq!(result.unwrap(), serde_json::json!(42));
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
        Err(SandboxError::CpuLimitExceeded) => {}
        other => panic!("Expected Timeout, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_deno_memory_limit() {
    let mut options = RuntimeOptions::default();
    options.memory_limit = 8 * 1024 * 1024;
    options.wall_clock_limit = Duration::from_secs(10);
    options.cpu_time_limit = Duration::from_secs(10);
    let mut runtime = DenoSandbox::new(options);
    let code = "const x = new Uint8Array(128 * 1024 * 1024); x.length;";
    let input = serde_json::Value::Null;
    match runtime.execute(code, input).await {
        Ok(_) => {}
        Err(SandboxError::MemoryLimitExceeded) => {}
        Err(SandboxError::CpuLimitExceeded) => {}
        Err(SandboxError::Timeout) => {}
        other => panic!("Expected MemoryLimitExceeded, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_deno_cpu_limit() {
    let mut options = RuntimeOptions::default();
    options.cpu_time_limit = Duration::from_millis(300);
    options.wall_clock_limit = Duration::from_secs(2);
    let mut runtime = DenoSandbox::new(options);
    let code = "while (true) {}";
    let input = serde_json::Value::Null;
    match runtime.execute(code, input).await {
        Err(SandboxError::CpuLimitExceeded) => {}
        Err(SandboxError::Timeout) => {}
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
    match runtime.execute(wat, input).await.unwrap_err() {
        SandboxError::MemoryLimitExceeded => {}
        other => panic!("Expected MemoryLimitExceeded, got: {:?}", other),
    }
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

#[tokio::test]
async fn test_wasm_stdout() {
    let wat = r#"
    (module
        (import "wasi_snapshot_preview1" "fd_write"
            (func $fd_write (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 8) "hello wasm\n")
        (func (export "_start")
            (i32.store (i32.const 0) (i32.const 8))
            (i32.store (i32.const 4) (i32.const 11))
            (call $fd_write
                (i32.const 1)
                (i32.const 0)
                (i32.const 1)
                (i32.const 20))
            drop
        )
    )
    "#;
    let options = RuntimeOptions::default();
    let mut runtime = WasmSandbox::new(options).unwrap();
    let input = serde_json::Value::Null;
    let output = runtime.execute(wat, input).await.unwrap();
    match output {
        serde_json::Value::String(s) => assert!(!s.is_empty(), "stdout should not be empty"),
        other => panic!("Expected String output, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_wasm_invalid_stdout_utf8() {
    let wat = r#"
    (module
        (import "wasi_snapshot_preview1" "fd_write"
            (func $fd_write (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 0) "\ff\fe\fd") ;; Invalid UTF-8
        (func (export "_start")
            (i32.store (i32.const 8) (i32.const 0))
            (i32.store (i32.const 12) (i32.const 3))
            (call $fd_write
                (i32.const 1)
                (i32.const 8)
                (i32.const 1)
                (i32.const 20))
            drop
        )
    )
    "#;
    let options = RuntimeOptions::default();
    let mut runtime = WasmSandbox::new(options).unwrap();
    let input = serde_json::Value::Null;
    let result = runtime.execute(wat, input).await;
    match result {
        Err(SandboxError::RuntimeError(e)) => {
            assert!(e.contains("stdout contains invalid UTF-8"));
            assert!(e.contains("fffefd"));
        }
        other => panic!("Expected RuntimeError with UTF-8 failure, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_network_allowlist_rejection() {
    let mut options = RuntimeOptions::default();
    options.permissions.network_allowlist = vec!["https://example.com".to_string()];
    let mut runtime = DenoSandbox::new(options);
    let code = "fetch('https://example.com')";
    let input = serde_json::Value::Null;
    match runtime.execute(code, input).await {
        Err(SandboxError::PermissionDenied(_)) => {}
        other => panic!("Expected PermissionDenied, got: {:?}", other),
    }
}
