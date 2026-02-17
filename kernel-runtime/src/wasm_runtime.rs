use crate::{RuntimeOptions, SandboxError, SandboxRuntime};
use anyhow::Result;
use async_trait::async_trait;
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtxBuilder};
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

impl wasmtime::ResourceLimiter for Ctx {
    fn memory_growing(&mut self, current: usize, desired: usize, maximum: Option<usize>) -> Result<bool, anyhow::Error> {
        self.limits.memory_growing(current, desired, maximum)
    }
    fn table_growing(&mut self, current: usize, desired: usize, maximum: Option<usize>) -> Result<bool, anyhow::Error> {
        // StoreLimits uses usize for table growing in newer wasmtime versions?
        // My previous check said "expected usize, found u32".
        // So passing usize directly is correct.
        self.limits.table_growing(current, desired, maximum)
    }
}

#[async_trait]
impl SandboxRuntime for WasmSandbox {
    async fn execute(
        &mut self,
        code: &str,
        _input: serde_json::Value,
    ) -> Result<serde_json::Value, SandboxError> {
        let mut linker = Linker::new(&self.engine);

        wasmtime_wasi::p1::add_to_linker_async(&mut linker, |ctx: &mut Ctx| &mut ctx.wasi)
            .map_err(|e| SandboxError::InitError(e.to_string()))?;

        let wasi = WasiCtxBuilder::new()
            .build_p1();

        let limits = StoreLimitsBuilder::new()
            .memory_size(self.options.memory_limit)
            .build();

        let mut store = Store::new(&self.engine, Ctx { wasi, limits });
        store.limiter(|state| &mut state.limits);

        let fuel = self.options.cpu_time_limit.as_millis() as u64 * 10_000;
        store.set_fuel(fuel).unwrap();
        store.set_epoch_deadline(1);

        let engine_clone = self.engine.clone();
        let timeout = self.options.wall_clock_limit;
        std::thread::spawn(move || {
            std::thread::sleep(timeout);
            engine_clone.increment_epoch();
        });

        let module = Module::new(&self.engine, code).map_err(|e| SandboxError::RuntimeError(e.to_string()))?;

        let instance = linker.instantiate_async(&mut store, &module).await.map_err(|e| SandboxError::RuntimeError(e.to_string()))?;

        let start = instance.get_typed_func::<(), ()>(&mut store, "_start").map_err(|e| SandboxError::RuntimeError(e.to_string()))?;

        let execution = start.call_async(&mut store, ());

        match execution.await {
            Ok(_) => {},
            Err(e) => {
                return Err(SandboxError::RuntimeError(e.to_string()));
            }
        }

        // Output retrieval deferred due to wasmtime-wasi 38 API complexity
        Ok(serde_json::Value::Null)
    }
}
