use async_trait::async_trait;
use std::time::Duration;
use thiserror::Error;

pub mod deno_runtime;
pub mod wasm_runtime;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("Execution timeout")]
    Timeout,
    #[error("Memory limit exceeded")]
    MemoryLimitExceeded,
    #[error("CPU limit exceeded")]
    CpuLimitExceeded,
    #[error("Runtime error: {0}")]
    RuntimeError(String),
    #[error("Initialization error: {0}")]
    InitError(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
}

#[derive(Debug, Clone)]
pub struct RuntimeOptions {
    pub memory_limit: usize, // bytes
    /// CPU budget for sandbox execution.
    ///
    /// Runtime-specific notes:
    /// - Wasm: enforced via wasmtime fuel.
    /// - Deno: enforced via process CPU time sampling.
    pub cpu_time_limit: Duration,
    pub wall_clock_limit: Duration,
    pub permissions: Permissions,
    pub max_output_size: Option<usize>,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            memory_limit: 128 * 1024 * 1024,
            cpu_time_limit: Duration::from_secs(5),
            wall_clock_limit: Duration::from_secs(30),
            permissions: Permissions::default(),
            max_output_size: Some(1 << 20),
        }
    }
}

/// Permissions settings for the sandbox.
#[derive(Debug, Clone, Default)]
pub struct Permissions {
    /// URL prefixes or domains for network access.
    ///
    /// [IMPORTANT] Enforcement is not yet implemented. Both DenoSandbox and
    /// WasmSandbox will return SandboxError::PermissionDenied if this vector is non-empty.
    pub network_allowlist: Vec<String>,
}

#[async_trait]
pub trait SandboxRuntime {
    /// Executes user-supplied code with JSON input and returns JSON output.
    ///
    /// Implementations must execute code and return the result as a JSON value.
    /// Runtimes should support asynchronous execution by awaiting any Promise or
    /// future produced by the user code before returning the output.
    async fn execute(
        &mut self,
        code: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, SandboxError>;
}
