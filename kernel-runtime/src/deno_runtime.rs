use crate::{RuntimeOptions, SandboxError, SandboxRuntime};
use async_trait::async_trait;
use deno_core::{JsRuntime, OpState, RuntimeOptions as DenoOptions, op2, v8};
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub struct DenoSandbox {
    options: RuntimeOptions,
}

impl DenoSandbox {
    pub fn new(options: RuntimeOptions) -> Self {
        Self { options }
    }
}

#[derive(Clone, Default)]
struct OutputState {
    value: serde_json::Value,
}

#[derive(Clone, Copy, Default)]
struct OutputConfig {
    max_output_size: Option<usize>,
}

struct SizeLimitedWriter {
    written: usize,
    limit: usize,
}

impl Write for SizeLimitedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let next = self.written.saturating_add(buf.len());
        if next > self.limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "output size limit exceeded (max: {} bytes, got: {}+ bytes)",
                    self.limit, self.limit
                ),
            ));
        }
        self.written = next;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[op2]
pub fn op_set_output(
    state: &mut OpState,
    #[serde] value: serde_json::Value,
) -> Result<(), std::io::Error> {
    if let Some(max_output_size) = state
        .try_borrow::<OutputConfig>()
        .and_then(|cfg| cfg.max_output_size)
    {
        let mut writer = SizeLimitedWriter {
            written: 0,
            limit: max_output_size,
        };
        serde_json::to_writer(&mut writer, &value).map_err(|err| {
            if let Some(io_err) = err.io_error_kind() {
                return std::io::Error::new(io_err, err.to_string());
            }
            std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
        })?;
    }

    state.put(OutputState { value });
    Ok(())
}

deno_core::extension!(sandbox_ext, ops = [op_set_output],);

#[async_trait]
impl SandboxRuntime for DenoSandbox {
    async fn execute(
        &mut self,
        code: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, SandboxError> {
        if !self.options.permissions.network_allowlist.is_empty() {
            return Err(SandboxError::PermissionDenied(
                "permissions.network_allowlist is not enforced yet".to_string(),
            ));
        }

        let options = self.options.clone();
        let code = code.to_string();

        let result = tokio::task::spawn_blocking(move || {
            // Reserve a bounded headroom within the configured limit so V8 can unwind safely
            // without allowing memory usage to exceed `options.memory_limit`.
            let near_heap_headroom = (options.memory_limit / 8).clamp(256 * 1024, 16 * 1024 * 1024);
            let initial_heap_limit = options
                .memory_limit
                .saturating_sub(near_heap_headroom)
                .max(1);

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| SandboxError::InitError(e.to_string()))?;

            rt.block_on(async move {
                // Use init() as determined by compiler
                let ext = sandbox_ext::init();

                let create_params = v8::CreateParams::default()
                    .heap_limits(0, initial_heap_limit);

                let mut js_runtime = JsRuntime::new(DenoOptions {
                    extensions: vec![ext],
                    create_params: Some(create_params),
                    ..Default::default()
                });
                js_runtime.op_state().borrow_mut().put(OutputConfig {
                    max_output_size: options.max_output_size,
                });

                let isolate_handle = js_runtime.v8_isolate().thread_safe_handle();
                let wall_clock_limit = options.wall_clock_limit;
                let cpu_wall_time_limit = options.cpu_time_limit;
                let cancelled = Arc::new(AtomicBool::new(false));
                let is_heap_oom = Arc::new(AtomicBool::new(false));
                let is_wall_timeout = Arc::new(AtomicBool::new(false));
                let is_cpu_wall_timeout = Arc::new(AtomicBool::new(false));

                let near_heap_isolate = isolate_handle.clone();
                let near_heap_flag = is_heap_oom.clone();
                js_runtime.add_near_heap_limit_callback(move |current_limit, _initial_limit| {
                    near_heap_flag.store(true, Ordering::SeqCst);
                    near_heap_isolate.terminate_execution();
                    current_limit
                        .saturating_add(near_heap_headroom)
                        .min(options.memory_limit)
                });

                let watchdog_cancel = cancelled.clone();
                let wall_timeout_flag = is_wall_timeout.clone();
                let wall_isolate_handle = isolate_handle.clone();
                std::thread::spawn(move || {
                    let sleep_slice = Duration::from_millis(10);
                    let started = std::time::Instant::now();
                    while started.elapsed() < wall_clock_limit {
                        if watchdog_cancel.load(Ordering::SeqCst) {
                            return;
                        }
                        std::thread::sleep(sleep_slice);
                    }
                    if !watchdog_cancel.load(Ordering::SeqCst) {
                        wall_timeout_flag.store(true, Ordering::SeqCst);
                        wall_isolate_handle.terminate_execution();
                    }
                });

                let cpu_cancel = cancelled.clone();
                let cpu_wall_timeout_flag = is_cpu_wall_timeout.clone();
                let cpu_isolate_handle = isolate_handle.clone();
                // This is a wall-clock timeout approximation, not actual CPU consumption metering; true CPU accounting is not implemented.
                std::thread::spawn(move || {
                    let sleep_slice = Duration::from_millis(10);
                    let started = std::time::Instant::now();
                    while started.elapsed() < cpu_wall_time_limit {
                        if cpu_cancel.load(Ordering::SeqCst) {
                            return;
                        }
                        std::thread::sleep(sleep_slice);
                    }
                    if !cpu_cancel.load(Ordering::SeqCst) {
                        cpu_wall_timeout_flag.store(true, Ordering::SeqCst);
                        cpu_isolate_handle.terminate_execution();
                    }
                });

                let input_json = serde_json::to_string(&input).expect("serialize input JSON must not fail");
                let setup_code = format!("globalThis.INPUT = {};", input_json);
                js_runtime
                    .execute_script("setup", setup_code)
                    .map_err(|e| SandboxError::RuntimeError(e.to_string()))?;

                let wrapped_code = format!(
                    r#"
                    (async function() {{
                        try {{
                            const result = eval({});
                            const resolved = await Promise.resolve(result);
                            Deno.core.ops.op_set_output(resolved === undefined ? null : resolved);
                        }} catch (error) {{
                            const message = error instanceof Error ? error.message : String(error);
                            Deno.core.ops.op_set_output({{ "__sandbox_error__": message }});
                            throw error;
                        }}
                    }})()
                "#,
                    serde_json::to_string(&code).expect("serialize sandboxed code to JSON must not fail")
                );

                let execution_future = async {
                    js_runtime
                        .execute_script("user_code", wrapped_code)
                        .map_err(|e| SandboxError::RuntimeError(e.to_string()))?;
                    js_runtime
                        .run_event_loop(Default::default())
                        .await
                        .map_err(|e| SandboxError::RuntimeError(e.to_string()))?;
                    Ok::<(), SandboxError>(())
                };

                let execution_result =
                    match tokio::time::timeout(options.wall_clock_limit, execution_future).await {
                        Ok(res) => res,
                        Err(_) => {
                            is_wall_timeout.store(true, Ordering::SeqCst);
                            isolate_handle.terminate_execution();
                            Err(SandboxError::Timeout)
                        }
                    };

                cancelled.store(true, Ordering::SeqCst);

                if is_heap_oom.load(Ordering::SeqCst) {
                    return Err(SandboxError::MemoryLimitExceeded);
                }

                if is_cpu_wall_timeout.load(Ordering::SeqCst) {
                    return Err(SandboxError::CpuLimitExceeded);
                }

                if is_wall_timeout.load(Ordering::SeqCst) {
                    return Err(SandboxError::Timeout);
                }

                match execution_result {
                    Ok(()) => {
                        let op_state = js_runtime.op_state();
                        let op_state = op_state.borrow();
                        if let Some(output) = op_state.try_borrow::<OutputState>() {
                            Ok(output.value.clone())
                        } else {
                            Ok(serde_json::Value::Null)
                        }
                    }
                    Err(err) => Err(err),
                }
            })
        })
        .await;

        match result {
            Ok(res) => res,
            Err(e) => Err(SandboxError::RuntimeError(e.to_string())),
        }
    }
}
