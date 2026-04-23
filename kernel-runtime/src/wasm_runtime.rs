use crate::{MAX_FETCH_BODY_BYTES, RuntimeOptions, SandboxError, SandboxRuntime};
use async_trait::async_trait;
use base64::Engine as _;
use futures_util::StreamExt;
use serde::Deserialize;
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
const MAX_FETCH_URL_BYTES: usize = 8 * 1024;
const MAX_FETCH_METHOD_BYTES: usize = 16;
const MAX_FETCH_HEADERS_JSON_BYTES: usize = 64 * 1024;
const MAX_FETCH_REQUEST_BODY_BYTES: usize = MAX_FETCH_BODY_BYTES;

// Error codes for flexi_fetch
// Table:
// -1  ERR_NO_MEMORY_EXPORT   -> guest memory export is missing
// -2  ERR_READ_MEM           -> failed to read guest memory
// -3  ERR_UTF8               -> UTF-8 decode failed
// -4  ERR_CHECK_URL          -> URL policy validation failed
// -5  (reserved)             -> historical gap, intentionally unused
// -6  ERR_REQUEST_FAIL       -> HTTP request failed
// -7  ERR_READ_BODY_FAIL     -> failed reading HTTP response body
// -8  (reserved)             -> historical gap, intentionally unused
// -9  ERR_WRITE_FAIL         -> failed writing response to guest memory
// -10 ERR_INVALID_LEN        -> invalid pointer/length pair from guest
// -11 ERR_PARSE_METHOD       -> invalid HTTP method
// -12 ERR_PARSE_HEADERS      -> invalid headers JSON format
// -13 ERR_RESPONSE_TOO_LARGE -> response exceeded MAX_FETCH_BODY_BYTES
const ERR_NO_MEMORY_EXPORT: i32 = -1;
const ERR_READ_MEM: i32 = -2;
const ERR_UTF8: i32 = -3;
const ERR_CHECK_URL: i32 = -4;
const ERR_REQUEST_FAIL: i32 = -6;
const ERR_READ_BODY_FAIL: i32 = -7;
const ERR_WRITE_FAIL: i32 = -9;
const ERR_INVALID_LEN: i32 = -10;
const ERR_PARSE_METHOD: i32 = -11;
const ERR_PARSE_HEADERS: i32 = -12;
const ERR_RESPONSE_TOO_LARGE: i32 = -13;

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

impl SyncWasi {
    fn new_checked(wasi: WasiP1Ctx) -> Self {
        assert_wasi_stream_sync_invariants();
        Self(wasi)
    }
}

fn assert_wasi_stream_sync_invariants() {
    fn assert_sync<T: Sync>() {}
    assert_sync::<MemoryInputPipe>();
    assert_sync::<MemoryOutputPipe>();
}

fn build_sandbox_wasi(
    input_json: String,
    stdout: MemoryOutputPipe,
    stderr: MemoryOutputPipe,
) -> WasiP1Ctx {
    // `SyncWasi` の unsafe 前提をここで固定する:
    // stdin/stdout/stderr は必ず `Memory*Pipe` を使い、型は `Sync` であることを
    // コンパイル時に検証する。
    assert_wasi_stream_sync_invariants();
    let mut wasi_builder = WasiCtxBuilder::new();
    wasi_builder.stdin(MemoryInputPipe::new(input_json.into_bytes()));
    wasi_builder.stdout(stdout);
    wasi_builder.stderr(stderr);
    wasi_builder.build_p1()
}

struct Ctx {
    wasi: SyncWasi,
    limits: StoreLimits,
    stdout: MemoryOutputPipe,
    client: reqwest::Client,
    allowlist: crate::NormalizedAllowlist,
    fetch_budget: Duration,
    fetch_started_at: Instant,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum HeaderInputValue {
    Single(String),
    Multi(Vec<String>),
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
    // String mapping fallback for wasmtime 42.0.2 diagnostics when no Trap is available.
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

                        let read_string =
                            |ptr: i32, len: i32, max_len: usize| -> Result<String, i32> {
                                if ptr < 0 || len < 0 {
                                    return Err(ERR_INVALID_LEN);
                                }
                                let start = ptr as usize;
                                let len = len as usize;
                                if len > max_len {
                                    return Err(ERR_INVALID_LEN);
                                }
                                // Check memory bounds before allocation
                                let end = start.checked_add(len).ok_or(ERR_INVALID_LEN)?;
                                if end > mem.data_size(&caller) {
                                    return Err(ERR_INVALID_LEN);
                                }

                                let mut buf = vec![0u8; len];
                                if mem.read(&caller, start, &mut buf).is_err() {
                                    return Err(ERR_READ_MEM);
                                }
                                String::from_utf8(buf).map_err(|_| ERR_UTF8)
                            };

                        let url_str = match read_string(url_ptr, url_len, MAX_FETCH_URL_BYTES) {
                            Ok(s) => s,
                            Err(e) => return Ok(e),
                        };

                        let method_str =
                            match read_string(method_ptr, method_len, MAX_FETCH_METHOD_BYTES) {
                                Ok(s) => s,
                                Err(e) => return Ok(e),
                            };

                        let headers_str = match read_string(
                            headers_ptr,
                            headers_len,
                            MAX_FETCH_HEADERS_JSON_BYTES,
                        ) {
                            Ok(s) => s,
                            Err(e) => return Ok(e),
                        };

                        let body_str =
                            match read_string(body_ptr, body_len, MAX_FETCH_REQUEST_BODY_BYTES) {
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

                        let elapsed = caller.data().fetch_started_at.elapsed();
                        let remaining = caller.data().fetch_budget.saturating_sub(elapsed);
                        if remaining.is_zero() {
                            return Ok(ERR_REQUEST_FAIL);
                        }

                        let mut builder = client.request(method, url).timeout(remaining);

                        if !headers_str.is_empty() {
                            let headers_val: std::collections::HashMap<String, HeaderInputValue> =
                                match serde_json::from_str(&headers_str) {
                                    Ok(v) => v,
                                    Err(_) => return Ok(ERR_PARSE_HEADERS),
                                };

                            for (k, v) in headers_val {
                                match v {
                                    HeaderInputValue::Single(s) => {
                                        builder = builder.header(k, s);
                                    }
                                    HeaderInputValue::Multi(values) => {
                                        for value in values {
                                            builder = builder.header(&k, value);
                                        }
                                    }
                                }
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

                        let body_base64 =
                            base64::engine::general_purpose::STANDARD.encode(&body_bytes);
                        let body_text = String::from_utf8(body_bytes).ok();
                        let json_resp = match body_text {
                            Some(body) => serde_json::json!({
                                "status": status,
                                "body": body,
                                "body_base64": body_base64
                            }),
                            None => serde_json::json!({
                                "status": status,
                                "body_base64": body_base64
                            }),
                        };
                        let json_str = json_resp.to_string();
                        let json_bytes = json_str.as_bytes();

                        // Check non-negative length for out buffer
                        if out_max_len < 0 {
                            return Ok(ERR_INVALID_LEN);
                        }
                        if out_ptr < 0 {
                            return Ok(ERR_INVALID_LEN);
                        }
                        let out_len = out_max_len as usize;
                        let out_start = out_ptr as usize;
                        let out_end = match out_start.checked_add(out_len) {
                            Some(end) => end,
                            None => return Ok(ERR_INVALID_LEN),
                        };
                        if out_end > mem.data_size(&caller) {
                            return Ok(ERR_INVALID_LEN);
                        }
                        if json_bytes.len() > out_len {
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
        let stderr = MemoryOutputPipe::new(stdout_capacity);
        let input_json = serde_json::to_string(&input)
            .map_err(|e| SandboxError::RuntimeError(format!("failed to serialize input: {e}")))?;
        let wasi = build_sandbox_wasi(input_json, stdout.clone(), stderr);

        let limits = StoreLimitsBuilder::new()
            .memory_size(self.options.memory_limit)
            .trap_on_grow_failure(true)
            .build();

        let resolver = Arc::new(crate::AllowlistResolver::new(
            &self.options.permissions.network_allowlist,
        ));
        let redirect_allowlist =
            crate::NormalizedAllowlist::new(&self.options.permissions.network_allowlist);
        let client = reqwest::Client::builder()
            .dns_resolver(resolver)
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() >= 10 {
                    return attempt.stop();
                }
                if crate::check_url(attempt.url().as_str(), &redirect_allowlist).is_ok() {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()
            .map_err(|e| SandboxError::InitError(e.to_string()))?;

        let mut store = Store::new(
            &self.engine,
            Ctx {
                wasi: SyncWasi::new_checked(wasi),
                limits,
                stdout,
                client,
                allowlist: crate::NormalizedAllowlist::new(
                    &self.options.permissions.network_allowlist,
                ),
                fetch_budget: self.options.wall_clock_limit,
                fetch_started_at: Instant::now(),
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
                Ok(Err(e)) => return Err(map_wasm_error(e.into())),
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
        {
            let ctx = store.data_mut();
            ctx.fetch_budget = execute_budget;
            ctx.fetch_started_at = Instant::now();
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
                return Err(map_wasm_error(e.into()));
            }
        };

        let start = match instance.get_typed_func::<(), ()>(&mut store, "_start") {
            Ok(start) => start,
            Err(e) => {
                self.stop_watchdog();
                return Err(map_wasm_error(e.into()));
            }
        };

        let execution = start.call_async(&mut store, ());

        match execution.await {
            Ok(_) => {}
            Err(e) => {
                self.stop_watchdog();
                let mapped = map_wasm_error(e.into());
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
