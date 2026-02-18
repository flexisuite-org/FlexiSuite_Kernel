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
    pub cpu_time_limit: Duration,
    pub wall_clock_limit: Duration,
    pub permissions: Permissions,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            memory_limit: 128 * 1024 * 1024,
            cpu_time_limit: Duration::from_secs(5),
            wall_clock_limit: Duration::from_secs(30),
            permissions: Permissions::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Permissions {
    // URL prefixes or domains.
    // Current runtimes reject execution when non-empty because allowlist enforcement
    // is not implemented yet.
    pub network_allowlist: Vec<String>,
}

#[async_trait]
pub trait SandboxRuntime {
    /// Executes user-supplied code with JSON input and returns JSON output.
    ///
    /// Deno execution supports asynchronous user code: the runtime awaits Promise
    /// results from the `wrapped_code`/`eval(...)` path before calling
    /// `op_set_output`.
    async fn execute(
        &mut self,
        code: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, SandboxError>;
}
