use crate::{RuntimeOptions, SandboxError, SandboxRuntime};
use async_trait::async_trait;
use futures_util::StreamExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, Trap};
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};

pub struct WasmSandbox {
    engine: Engine,
    options: RuntimeOptions,
    watchdog: Option<(Arc<AtomicBool>, std::thread::JoinHandle<()>)>,
    compile_limiter: Arc<Semaphore>,
}

const DEFAULT_MAX_STDOUT: usize = 1 << 20; // 1MB default hard cap
const MAX_FETCH_BODY_BYTES: usize = 10 * 1024 * 1024; // 10MB

// Error codes for flexi_fetch
const ERR_NO_MEMORY_EXPORT: i32 = -1;
const ERR_READ_MEM: i32 = -2;
const ERR_UTF8: i32 = -3;
const ERR_CHECK_URL: i32 = -4;
const ERR_REQUEST_FAIL: i32 = -6; // Skip -5 (unused)
const ERR_READ_BODY_FAIL: i32 = -7;
const ERR_WRITE_FAIL: i32 = -9; // Skip -8
const ERR_INVALID_LEN: i32 = -10;
const ERR_PARSE_METHOD: i32 = -11;
const ERR_PARSE_HEADERS: i32 = -12;
const ERR_RESPONSE_TOO_LARGE: i32 = -13;
const ERR_RESPONSE_UTF8: i32 = -14;

impl Drop for WasmSandbox {
    fn drop(&mut self) {
        self.stop_watchdog();
    }
}

impl WasmSandbox {
    fn stop_watchdog(&mut self) {
        if let Some((cancel, handle)) = self.watchdog.take() {
            cancel.store(true, Ordering::SeqCst);
            let _ = handle.join();
        }
    }

    pub fn new(options: RuntimeOptions) -> Result<Self, SandboxError> {
        let mut config = Config::new();
        config.async_support(true);
        config.consume_fuel(true);
        config.epoch_interruption(true);

        let engine = Engine::new(&config).map_err(|e| SandboxError::InitError(e.to_string()))?;
        let compile_permits = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(2, 8);

        Ok(Self {
            engine,
            options,
            watchdog: None,
            compile_limiter: Arc::new(Semaphore::new(compile_permits)),
        })
    }
}

struct SyncWasi(pub WasiP1Ctx);
/// # Safety
///
/// `WasiP1Ctx` contains `Box<dyn StdinStream>` which is `!Sync` by trait definition.
/// However, `WasmSandbox` only injects `MemoryInputPipe` and `MemoryOutputPipe` which are `Sync`.
/// Additionally, `Store` is `!Sync` and `!Send` (unless `T: Send`), ensuring single-threaded access to `Ctx`
/// within the store. `SyncWasi` wrapper is used to satisfy `Sync` bounds required by
/// `func_wrap_async` / `add_to_linker_async` when using `async` runtimes, but access is guarded
/// by the single-threaded nature of the `Store` and `Engine` usage in `execute`.
///
/// **Invariant:** `WasiCtxBuilder` MUST NOT be configured with non-Sync streams (e.g. file streams)
/// when using this wrapper.
unsafe impl Sync for SyncWasi {}

struct Ctx {
    wasi: SyncWasi,
    limits: StoreLimits,
    stdout: MemoryOutputPipe,
    client: reqwest::Client,
    allowlist: Vec<String>,
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
    if message.contains("allocation too large")
        || message.contains("exceeded memory limits")
        || message.contains("growing memory")
    {
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
        input: serde_json::Value,
    ) -> Result<serde_json::Value, SandboxError> {
        self.stop_watchdog();
        let started_at = Instant::now();

        let mut linker = Linker::new(&self.engine);

        linker
            .func_wrap_async(
                "env",
                "flexi_fetch",
                |mut caller: wasmtime::Caller<'_, Ctx>,
                 (
                    url_ptr,
                    url_len,
                    method_ptr,
                    method_len,
                    headers_ptr,
                    headers_len,
                    body_ptr,
                    body_len,
                    out_ptr,
                    out_max_len,
                ): (i32, i32, i32, i32, i32, i32, i32, i32, i32, i32)| {
                    Box::new(async move {
                        let mem = match caller.get_export("memory") {
                            Some(wasmtime::Extern::Memory(m)) => m,
                            _ => return Ok(ERR_NO_MEMORY_EXPORT),
                        };

                        let read_string = |ptr: i32, len: i32| -> Result<String, i32> {
                            if len < 0 {
                                return Err(ERR_INVALID_LEN);
                            }
                            // Check memory bounds before allocation
                            let end = (ptr as usize).checked_add(len as usize).ok_or(ERR_INVALID_LEN)?;
                            if end > mem.data_size(&caller) {
                                return Err(ERR_READ_MEM);
                            }

                            let mut buf = vec![0u8; len as usize];
                            if mem.read(&caller, ptr as usize, &mut buf).is_err() {
                                return Err(ERR_READ_MEM);
                            }
                            String::from_utf8(buf).map_err(|_| ERR_UTF8)
                        };

                        let url_str = match read_string(url_ptr, url_len) {
                            Ok(s) => s,
                            Err(e) => return Ok(e),
                        };

                        let method_str = match read_string(method_ptr, method_len) {
                            Ok(s) => s,
                            Err(e) => return Ok(e),
                        };

                        let headers_str = match read_string(headers_ptr, headers_len) {
                            Ok(s) => s,
                            Err(e) => return Ok(e),
                        };

                        let body_str = match read_string(body_ptr, body_len) {
                            Ok(s) => s,
                            Err(e) => return Ok(e),
                        };

                        let client = caller.data().client.clone();
                        let allowlist = caller.data().allowlist.clone();

                        // Initial check to block IP literals if not explicitly allowed
                        let url = match crate::check_url(&url_str, &allowlist) {
                            Ok(u) => u,
                            Err(_) => return Ok(ERR_CHECK_URL),
                        };

                        let method = match method_str.parse::<reqwest::Method>() {
                            Ok(m) => m,
                            Err(_) => return Ok(ERR_PARSE_METHOD),
                        };

                        let mut builder = client.request(method, url);

                        if !headers_str.is_empty() {
                            // Note: We deserialize to HashMap<String, String>, which means if the input JSON
                            // contained duplicate keys (multi-value headers), they might be lost or overwritten
                            // depending on serde_json behavior. Wasm callers should pre-join multi-value headers
                            // (e.g. "Header: val1, val2") or we need a more complex structure.
                            // For this iteration, we accept this limitation.
                            let headers: std::collections::HashMap<String, String> =
                                match serde_json::from_str(&headers_str) {
                                    Ok(h) => h,
                                    Err(_) => return Ok(ERR_PARSE_HEADERS),
                                };
                            for (k, v) in headers {
                                builder = builder.header(k, v);
                            }
                        }

                        if !body_str.is_empty() {
                            builder = builder.body(body_str);
                        }

                        let resp = match builder.send().await {
                            Ok(r) => r,
                            Err(_) => return Ok(ERR_REQUEST_FAIL),
                        };

                        let status = resp.status().as_u16();

                        // Streaming read with limit
                        let mut body_bytes = Vec::new();
                        let mut stream = resp.bytes_stream();
                        while let Some(chunk_res) = stream.next().await {
                            let chunk = match chunk_res {
                                Ok(c) => c,
                                Err(_) => return Ok(ERR_READ_BODY_FAIL),
                            };
                            if body_bytes.len() + chunk.len() > MAX_FETCH_BODY_BYTES {
                                return Ok(ERR_RESPONSE_TOO_LARGE);
                            }
                            body_bytes.extend_from_slice(&chunk);
                        }

                        let body = match String::from_utf8(body_bytes) {
                            Ok(s) => s,
                            Err(_) => return Ok(ERR_RESPONSE_UTF8),
                        };

                        let json_resp = serde_json::json!({
                            "status": status,
                            "body": body
                        });
                        let json_str = json_resp.to_string();
                        let json_bytes = json_str.as_bytes();

                        // Check non-negative length for out buffer
                        if out_max_len < 0 {
                            return Ok(ERR_RESPONSE_TOO_LARGE);
                        }
                        if json_bytes.len() > out_max_len as usize {
                            return Ok(ERR_RESPONSE_TOO_LARGE);
                        }

                        if mem
                            .write(&mut caller, out_ptr as usize, json_bytes)
                            .is_err()
                        {
                            return Ok(ERR_WRITE_FAIL);
                        }

                        Ok(json_bytes.len() as i32)
                    })
                },
            )
            .map_err(|e| SandboxError::InitError(e.to_string()))?;

        wasmtime_wasi::p1::add_to_linker_async(&mut linker, |ctx: &mut Ctx| &mut ctx.wasi.0)
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
            .trap_on_grow_failure(true)
            .build();

        let resolver = Arc::new(crate::AllowlistResolver::new(
            self.options.permissions.network_allowlist.clone(),
        ));
        let client = reqwest::Client::builder()
            .dns_resolver(resolver)
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| SandboxError::InitError(e.to_string()))?;

        let mut store = Store::new(
            &self.engine,
            Ctx {
                wasi: SyncWasi(wasi),
                limits,
                stdout,
                client,
                allowlist: self.options.permissions.network_allowlist.clone(),
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

        let compile_queue_budget = self
            .options
            .wall_clock_limit
            .checked_sub(started_at.elapsed())
            .unwrap_or(Duration::ZERO);
        if compile_queue_budget.is_zero() {
            return Err(SandboxError::Timeout);
        }

        let compile_permit = match tokio::time::timeout(
            compile_queue_budget,
            self.compile_limiter.clone().acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => {
                return Err(SandboxError::InitError(
                    "Wasm compile semaphore is closed".to_string(),
                ));
            }
            Err(_) => return Err(SandboxError::Timeout),
        };

        let compile_budget = self
            .options
            .wall_clock_limit
            .checked_sub(started_at.elapsed())
            .unwrap_or(Duration::ZERO);
        if compile_budget.is_zero() {
            return Err(SandboxError::Timeout);
        }

        let compile_engine = self.engine.clone();
        let compile_code = code.to_string();
        // Keep permit for the entire compile phase to avoid admitting unbounded
        // concurrent compiles while this task is active.
        let _compile_permit = compile_permit;
        let module =
            match tokio::task::spawn_blocking(move || Module::new(&compile_engine, compile_code))
                .await
            {
                Ok(Ok(module)) => module,
                Ok(Err(e)) => return Err(map_wasm_error(e)),
                Err(e) => return Err(SandboxError::RuntimeError(e.to_string())),
            };

        if started_at.elapsed() > self.options.wall_clock_limit {
            return Err(SandboxError::Timeout);
        }

        let execute_budget = self
            .options
            .wall_clock_limit
            .checked_sub(started_at.elapsed())
            .unwrap_or(Duration::ZERO);
        if execute_budget.is_zero() {
            return Err(SandboxError::Timeout);
        }
        let engine_clone = self.engine.clone();
        let cancel_watchdog = Arc::new(AtomicBool::new(false));
        let is_wall_timeout = Arc::new(AtomicBool::new(false));
        let watchdog_cancel = cancel_watchdog.clone();
        let wall_timeout_flag = is_wall_timeout.clone();
        let watchdog_handle = std::thread::spawn(move || {
            let sleep_slice = Duration::from_millis(10);
            let started = Instant::now();
            while started.elapsed() < execute_budget {
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

        let instance = match linker.instantiate_async(&mut store, &module).await {
            Ok(instance) => instance,
            Err(e) => {
                self.stop_watchdog();
                return Err(map_wasm_error(e));
            }
        };

        let start = match instance.get_typed_func::<(), ()>(&mut store, "_start") {
            Ok(start) => start,
            Err(e) => {
                self.stop_watchdog();
                return Err(map_wasm_error(e));
            }
        };

        let execution = start.call_async(&mut store, ());

        match execution.await {
            Ok(_) => {}
            Err(e) => {
                self.stop_watchdog();
                let mapped = map_wasm_error(e);
                if matches!(mapped, SandboxError::RuntimeError(_))
                    && is_wall_timeout.load(Ordering::SeqCst)
                {
                    return Err(SandboxError::Timeout);
                }
                return Err(mapped);
            }
        }

        self.stop_watchdog();

        let stdout_bytes = store.data().stdout.contents();
        if stdout_bytes.len() > effective_max_stdout {
            return Err(SandboxError::RuntimeError(format!(
                "stdout limit exceeded (max: {} bytes). Output may be truncated.",
                effective_max_stdout
            )));
        }
        let output = String::from_utf8(stdout_bytes.to_vec()).map_err(|e| {
            let total_len = stdout_bytes.len();
            let truncated = total_len > 256;
            let hex_preview: String = stdout_bytes
                .iter()
                .take(256)
                .map(|b| format!("{:02x}", b))
                .collect();

            SandboxError::RuntimeError(format!(
                "stdout contains invalid UTF-8: {}. Raw bytes (hex, total {} bytes{}): {}{}",
                e,
                total_len,
                if truncated { ", truncated" } else { "" },
                hex_preview,
                if truncated { "..." } else { "" }
            ))
        })?;

        let trimmed = output.trim();
        if trimmed.is_empty() {
            return Ok(serde_json::Value::Null);
        }

        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(json) => Ok(json),
            Err(_) => Ok(serde_json::Value::String(trimmed.to_string())),
        }
    }
}
