#![allow(clippy::field_reassign_with_default)]

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
    assert!(
        result.is_ok(),
        "Deno async execution failed: {:?}",
        result.err()
    );
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
    match runtime.execute(wat, input).await {
        Err(SandboxError::Timeout) => {}
        Err(SandboxError::CpuLimitExceeded) => {}
        other => panic!("Expected Timeout, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_wasm_zero_wall_clock_limit_times_out_before_compile() {
    let wat = r#"
    (module
        (func (export "_start")
            nop
        )
    )
    "#;
    let mut options = RuntimeOptions::default();
    options.wall_clock_limit = Duration::ZERO;
    let mut runtime = WasmSandbox::new(options).unwrap();
    let input = serde_json::Value::Null;
    match runtime.execute(wat, input).await {
        Err(SandboxError::Timeout) => {}
        other => panic!(
            "Expected Timeout for zero wall clock limit, got: {:?}",
            other
        ),
    }
}

#[tokio::test]
async fn test_deno_invalid_memory_limit_is_init_error() {
    let mut options = RuntimeOptions::default();
    options.memory_limit = 8 * 1024 * 1024;
    let mut runtime = DenoSandbox::new(options);
    let code = "1 + 1";
    let input = serde_json::Value::Null;
    match runtime.execute(code, input).await {
        Err(SandboxError::InitError(e)) => {
            assert!(e.contains("below the minimum Deno/V8 heap limit"));
        }
        other => panic!(
            "Expected InitError for invalid memory limit, got: {:?}",
            other
        ),
    }
}

#[tokio::test]
async fn test_deno_min_memory_limit_allows_execution() {
    let mut options = RuntimeOptions::default();
    options.memory_limit = 16 * 1024 * 1024;
    let mut runtime = DenoSandbox::new(options);
    let code = "1 + 1";
    let input = serde_json::Value::Null;
    let result = runtime.execute(code, input).await;
    assert!(
        result.is_ok(),
        "Expected execution to succeed at minimum Deno heap limit, got: {:?}",
        result
    );
    assert_eq!(result.unwrap(), serde_json::json!(2));
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
        other => panic!("Expected CpuLimitExceeded, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_deno_output_limit() {
    let mut options = RuntimeOptions::default();
    options.max_output_size = Some(5);
    let mut runtime = DenoSandbox::new(options);
    let code = "'123456'";
    let input = serde_json::Value::Null;
    match runtime.execute(code, input).await {
        Err(SandboxError::RuntimeError(_)) => {}
        other => panic!("Expected RuntimeError for output limit, got: {:?}", other),
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
        SandboxError::RuntimeError(e) => {
            assert!(
                e.contains("minimum page size")
                    || e.contains("exceeds the limit")
                    || e.contains("memory out of bounds")
            );
        }
        other => panic!(
            "Expected MemoryLimitExceeded or RuntimeError, got: {:?}",
            other
        ),
    }
}

#[tokio::test]
async fn test_wasm_memory_oob_runtime() {
    let wat = r#"
    (module
        (memory 1)
        (func (export "_start")
            (i32.store (i32.const 70000) (i32.const 1))
        )
    )
    "#;
    let options = RuntimeOptions::default();
    let mut runtime = WasmSandbox::new(options).unwrap();
    let input = serde_json::Value::Null;
    match runtime.execute(wat, input).await.unwrap_err() {
        SandboxError::RuntimeError(e) => {
            assert!(e.contains("memory out of bounds"));
        }
        other => panic!("Expected RuntimeError for OOB, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_wasm_stdout_truncation() {
    let wat = r#"
    (module
        (import "wasi_snapshot_preview1" "fd_write"
            (func $fd_write (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 0) "1234567890")
        (func (export "_start")
            (i32.store (i32.const 16) (i32.const 0))
            (i32.store (i32.const 20) (i32.const 10))
            (call $fd_write
                (i32.const 1)
                (i32.const 16)
                (i32.const 1)
                (i32.const 32))
            drop
        )
    )
    "#;
    let mut options = RuntimeOptions::default();
    options.max_output_size = Some(5);
    let mut runtime = WasmSandbox::new(options).unwrap();
    let input = serde_json::Value::Null;
    match runtime.execute(wat, input).await.unwrap_err() {
        SandboxError::RuntimeError(e) => {
            assert!(
                e.contains("stdout limit exceeded"),
                "Error message should contain 'stdout limit exceeded', got: {}",
                e
            );
        }
        other => panic!("Expected RuntimeError for truncation, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_wasm_stdout_exact_limit_allowed() {
    let wat = r#"
    (module
        (import "wasi_snapshot_preview1" "fd_write"
            (func $fd_write (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 0) "12345")
        (func (export "_start")
            (i32.store (i32.const 16) (i32.const 0))
            (i32.store (i32.const 20) (i32.const 5))
            (call $fd_write
                (i32.const 1)
                (i32.const 16)
                (i32.const 1)
                (i32.const 32))
            drop
        )
    )
    "#;
    let mut options = RuntimeOptions::default();
    options.max_output_size = Some(5);
    let mut runtime = WasmSandbox::new(options).unwrap();
    let input = serde_json::Value::Null;
    let output = runtime.execute(wat, input).await.unwrap();
    assert_eq!(output, serde_json::json!(12345));
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
async fn test_wasm_invalid_wat_does_not_block_next_execution() {
    let mut options = RuntimeOptions::default();
    options.wall_clock_limit = Duration::from_secs(2);
    let mut runtime = WasmSandbox::new(options).unwrap();

    let invalid_wat = "(module (func";
    let input = serde_json::Value::Null;
    assert!(runtime.execute(invalid_wat, input.clone()).await.is_err());

    let valid_wat = r#"
    (module
        (func (export "_start")
            nop
        )
    )
    "#;
    let second =
        tokio::time::timeout(Duration::from_secs(1), runtime.execute(valid_wat, input)).await;
    assert!(
        second.is_ok(),
        "second execution timed out, watchdog cleanup may be leaking between runs"
    );
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
async fn test_wasm_json_stdout() {
    let wat = r#"
    (module
        (import "wasi_snapshot_preview1" "fd_write"
            (func $fd_write (param i32 i32 i32 i32) (result i32)))
        (memory (export "memory") 1)
        (data (i32.const 8) "42")
        (func (export "_start")
            (i32.store (i32.const 0) (i32.const 8))
            (i32.store (i32.const 4) (i32.const 2))
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
    assert_eq!(output, serde_json::json!(42));
}

#[tokio::test]
async fn test_wasm_non_json_stdout_returns_string() {
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
    assert_eq!(output, serde_json::json!("hello wasm"));
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
async fn test_wasm_network_allowlist_allows_execution_when_unused() {
    let mut options = RuntimeOptions::default();
    options.permissions.network_allowlist = vec!["https://example.com".to_string()];

    let mut wasm_runtime = WasmSandbox::new(options).unwrap();
    let wat = r#"
    (module
        (func (export "_start")
            nop
        )
    )
    "#;
    let input = serde_json::Value::Null;
    let result = wasm_runtime.execute(wat, input).await;
    assert!(
        result.is_ok(),
        "Expected execution to succeed when network is unused, got: {:?}",
        result
    );
}

#[tokio::test]
async fn test_deno_network_allowlist_allows_execution_when_unused() {
    let mut options = RuntimeOptions::default();
    options.permissions.network_allowlist = vec!["https://example.com".to_string()];

    let mut deno_runtime = DenoSandbox::new(options);
    let input = serde_json::Value::Null;
    let result = deno_runtime.execute("1 + 1", input).await;
    assert!(
        result.is_ok(),
        "Expected execution to succeed when network is unused, got: {:?}",
        result
    );
}
