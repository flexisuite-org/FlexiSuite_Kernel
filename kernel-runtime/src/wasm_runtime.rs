use crate::{RuntimeOptions, SandboxError, SandboxRuntime};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, Trap};
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::p1::WasiP1Ctx;

pub struct WasmSandbox {
    engine: Engine,
    options: RuntimeOptions,
}

impl WasmSandbox {
    pub fn new(options: RuntimeOptions) -> Result<Self, SandboxError> {
        let mut config = Config::new();
        config.async_support(true);
        config.consume_fuel(true);
        config.epoch_interruption(true);

        let engine = Engine::new(&config).map_err(|e| SandboxError::InitError(e.to_string()))?;

        Ok(Self { engine, options })
    }
}

struct Ctx {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
}

fn map_wasm_error(error: anyhow::Error) -> SandboxError {
    if let Some(trap) = error.downcast_ref::<Trap>() {
        return match trap {
            Trap::OutOfFuel => SandboxError::CpuLimitExceeded,
            Trap::Interrupt => SandboxError::Timeout,
            Trap::AllocationTooLarge | Trap::MemoryOutOfBounds => SandboxError::MemoryLimitExceeded,
            _ => SandboxError::RuntimeError(error.to_string()),
        };
    }

    let message = error.to_string();
    if message.contains("memory") && message.contains("limit") {
        SandboxError::MemoryLimitExceeded
    } else {
        SandboxError::RuntimeError(message)
    }
}

#[async_trait]
impl SandboxRuntime for WasmSandbox {
    async fn execute(
        &mut self,
        code: &str,
        _input: serde_json::Value,
    ) -> Result<serde_json::Value, SandboxError> {
        if !self.options.permissions.network_allowlist.is_empty() {
            return Err(SandboxError::PermissionDenied(
                "permissions.network_allowlist is not enforced yet".to_string(),
            ));
        }

        let mut linker = Linker::new(&self.engine);

        wasmtime_wasi::p1::add_to_linker_async(&mut linker, |ctx: &mut Ctx| &mut ctx.wasi)
            .map_err(|e| SandboxError::InitError(e.to_string()))?;

        let wasi = WasiCtxBuilder::new().build_p1();

        let limits = StoreLimitsBuilder::new()
            .memory_size(self.options.memory_limit)
            .build();

        let mut store = Store::new(&self.engine, Ctx { wasi, limits });
        store.limiter(|state| &mut state.limits);

        let fuel = self.options.cpu_time_limit.as_millis() as u64 * 10_000;
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
        std::thread::spawn(move || {
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

        let compile_engine = self.engine.clone();
        let compile_code = code.to_string();
        let module =
            tokio::task::spawn_blocking(move || Module::new(&compile_engine, compile_code))
                .await
                .map_err(|e| SandboxError::RuntimeError(e.to_string()))?
                .map_err(map_wasm_error)?;

        let instance = linker
            .instantiate_async(&mut store, &module)
            .await
            .map_err(map_wasm_error)?;

        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .map_err(map_wasm_error)?;

        let execution = start.call_async(&mut store, ());

        match execution.await {
            Ok(_) => {}
            Err(e) => {
                cancel_watchdog.store(true, Ordering::SeqCst);
                if is_wall_timeout.load(Ordering::SeqCst) {
                    return Err(SandboxError::Timeout);
                }
                return Err(map_wasm_error(e));
            }
        }

        cancel_watchdog.store(true, Ordering::SeqCst);

        // Stdout capture is not wired yet in this sandbox, so execution currently
        // returns Null even on wasmtime-wasi 41.0.3.
        Ok(serde_json::Value::Null)
    }
}
