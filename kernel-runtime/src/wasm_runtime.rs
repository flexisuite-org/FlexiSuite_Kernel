use crate::{RuntimeOptions, SandboxError, SandboxRuntime};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, Trap};
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};

pub struct WasmSandbox {
    engine: Engine,
    options: RuntimeOptions,
    watchdog: Option<(Arc<AtomicBool>, std::thread::JoinHandle<()>)>,
}

const DEFAULT_MAX_STDOUT: usize = 1 << 20; // 1MB default hard cap

impl Drop for WasmSandbox {
    fn drop(&mut self) {
        if let Some((cancel, handle)) = self.watchdog.take() {
            cancel.store(true, Ordering::SeqCst);
            let _ = handle.join();
        }
    }
}

impl WasmSandbox {
    pub fn new(options: RuntimeOptions) -> Result<Self, SandboxError> {
        let mut config = Config::new();
        config.async_support(true);
        config.consume_fuel(true);
        config.epoch_interruption(true);

        let engine = Engine::new(&config).map_err(|e| SandboxError::InitError(e.to_string()))?;

        Ok(Self {
            engine,
            options,
            watchdog: None,
        })
    }
}

struct Ctx {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
    stdout: MemoryOutputPipe,
}

fn map_wasm_error(error: anyhow::Error) -> SandboxError {
    if let Some(trap) = error.downcast_ref::<Trap>() {
        return match trap {
            Trap::OutOfFuel => SandboxError::CpuLimitExceeded,
            Trap::Interrupt => SandboxError::Timeout,
            Trap::AllocationTooLarge => SandboxError::MemoryLimitExceeded,
            Trap::MemoryOutOfBounds => {
                SandboxError::RuntimeError("memory out of bounds".to_string())
            }
            _ => SandboxError::RuntimeError(error.to_string()),
        };
    }

    let message = error.to_string();
    // String mapping fallback for wasmtime 41.0.3 diagnostics when no Trap is available.
    if message.contains("allocation too large") || message.contains("exceeded memory limits") {
        SandboxError::MemoryLimitExceeded
    } else if message.contains("memory out of bounds")
        || message.contains("index out of bounds")
        || message.contains("offset out of bounds")
    {
        SandboxError::RuntimeError(message)
    } else {
        SandboxError::RuntimeError(message)
    }
}

#[async_trait]
impl SandboxRuntime for WasmSandbox {
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

        if let Some((cancel, handle)) = self.watchdog.take() {
            cancel.store(true, Ordering::SeqCst);
            let _ = handle.join();
        }

        let mut linker = Linker::new(&self.engine);

        wasmtime_wasi::p1::add_to_linker_async(&mut linker, |ctx: &mut Ctx| &mut ctx.wasi)
            .map_err(|e| SandboxError::InitError(e.to_string()))?;

        let effective_max_stdout = self.options.max_output_size.unwrap_or(DEFAULT_MAX_STDOUT);
        let stdout_capacity = effective_max_stdout.saturating_add(1);
        let stdout = MemoryOutputPipe::new(stdout_capacity);
        let input_json = serde_json::to_string(&input)
            .map_err(|e| SandboxError::RuntimeError(format!("failed to serialize input: {e}")))?;
        let mut wasi_builder = WasiCtxBuilder::new();
        wasi_builder.stdin(MemoryInputPipe::new(input_json.into_bytes()));
        wasi_builder.stdout(stdout.clone());
        let wasi = wasi_builder.build_p1();

        let limits = StoreLimitsBuilder::new()
            .memory_size(self.options.memory_limit)
            .build();

        let mut store = Store::new(
            &self.engine,
            Ctx {
                wasi,
                limits,
                stdout,
            },
        );
        store.limiter(|state| &mut state.limits);

        let fuel = self
            .options
            .cpu_time_limit
            .as_millis()
            .saturating_mul(10_000)
            .min(u128::from(u64::MAX)) as u64;
        store
            .set_fuel(fuel)
            .map_err(|e| SandboxError::InitError(e.to_string()))?;
        store.set_epoch_deadline(1);
        store.epoch_deadline_trap();
        let engine_clone = self.engine.clone();
        let timeout = self.options.wall_clock_limit;
        let cancel_watchdog = Arc::new(AtomicBool::new(false));
        let is_wall_timeout = Arc::new(AtomicBool::new(false));
        let watchdog_cancel = cancel_watchdog.clone();
        let wall_timeout_flag = is_wall_timeout.clone();
        let watchdog_handle = std::thread::spawn(move || {
            let sleep_slice = Duration::from_millis(10);
            let started = std::time::Instant::now();
            while started.elapsed() < timeout {
                if watchdog_cancel.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(sleep_slice);
            }
            if !watchdog_cancel.load(Ordering::SeqCst) {
                wall_timeout_flag.store(true, Ordering::SeqCst);
                engine_clone.increment_epoch();
            }
        });
        self.watchdog = Some((cancel_watchdog, watchdog_handle));

        let stop_watchdog = |s: &mut Self| {
            if let Some((cancel, handle)) = s.watchdog.take() {
                cancel.store(true, Ordering::SeqCst);
                let _ = handle.join();
            }
        };

        let compile_engine = self.engine.clone();
        let compile_code = code.to_string();
        let module =
            match tokio::task::spawn_blocking(move || Module::new(&compile_engine, compile_code))
                .await
            {
                Ok(Ok(module)) => module,
                Ok(Err(e)) => {
                    stop_watchdog(self);
                    return Err(map_wasm_error(e.into()));
                }
                Err(e) => {
                    stop_watchdog(self);
                    return Err(SandboxError::RuntimeError(e.to_string()));
                }
            };

        let instance = match linker.instantiate_async(&mut store, &module).await {
            Ok(instance) => instance,
            Err(e) => {
                stop_watchdog(self);
                return Err(map_wasm_error(e));
            }
        };

        let start = match instance.get_typed_func::<(), ()>(&mut store, "_start") {
            Ok(start) => start,
            Err(e) => {
                stop_watchdog(self);
                return Err(map_wasm_error(e));
            }
        };

        let execution = start.call_async(&mut store, ());

        match execution.await {
            Ok(_) => {}
            Err(e) => {
                stop_watchdog(self);
                if let Some(msg) = e.downcast_ref::<Trap>().map(|t| t.to_string()).or_else(|| Some(e.to_string())) {
                    if msg.contains("write beyond capacity of MemoryOutputPipe") {
                        return Err(SandboxError::RuntimeError(format!(
                            "stdout limit exceeded (max: {} bytes). Output may be truncated.",
                            effective_max_stdout
                        )));
                    }
                }
                let mapped = map_wasm_error(e);
                if matches!(mapped, SandboxError::RuntimeError(_)) {
                    if is_wall_timeout.load(Ordering::SeqCst) {
                        return Err(SandboxError::Timeout);
                    }
                }
                return Err(mapped);
            }
        }

        stop_watchdog(self);

        let stdout_bytes = store.data().stdout.contents();
        if stdout_bytes.len() > effective_max_stdout {
            return Err(SandboxError::RuntimeError(format!(
                "stdout limit exceeded (max: {} bytes). Output may be truncated.",
                effective_max_stdout
            )));
        }
        let output = String::from_utf8(stdout_bytes.to_vec()).map_err(|e| {
            SandboxError::RuntimeError(format!(
                "stdout contains invalid UTF-8: {}. Raw bytes (hex): {}",
                e,
                stdout_bytes
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<String>()
            ))
        })?;
        Ok(serde_json::Value::String(output))
    }
}
