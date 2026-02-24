use crate::{RuntimeOptions, SandboxError, SandboxRuntime};
use async_trait::async_trait;
use deno_core::{JsRuntime, OpState, RuntimeOptions as DenoOptions, op2, v8};
use deno_error::JsErrorBox;
use futures_util::StreamExt;
#[cfg(unix)]
use libc::{clock_gettime, pthread_getcpuclockid, pthread_self, timespec};
use reqwest::{Client, Method};
use std::collections::HashMap;
use std::io::{Error, ErrorKind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::Semaphore;
use url::Url;

pub struct DenoSandbox {
    options: RuntimeOptions,
    exec_limiter: Arc<Semaphore>,
}

const DEFAULT_MAX_OUTPUT_SIZE: usize = 1 << 20; // 1MB default hard cap
const MAX_FETCH_BODY_BYTES: usize = 10 * 1024 * 1024; // 10MB
const MIN_DENO_HEAP_LIMIT: usize = 16 * 1024 * 1024; // avoid V8 process-abort range for tiny heaps

impl DenoSandbox {
    pub fn new(options: RuntimeOptions) -> Self {
        let permits = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(2, 8);
        Self {
            options,
            exec_limiter: Arc::new(Semaphore::new(permits)),
        }
    }
}

struct WatchdogGroup {
    cancelled: Arc<AtomicBool>,
    handles: Vec<JoinHandle<()>>,
}

impl WatchdogGroup {
    fn new(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled,
            handles: Vec::new(),
        }
    }

    fn spawn(&mut self, handle: JoinHandle<()>) {
        self.handles.push(handle);
    }
}

impl Drop for WatchdogGroup {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
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

#[derive(Clone)]
struct NetworkConfig {
    allowlist: Vec<String>,
}

fn check_url(url_str: &str, allowlist: &[String]) -> Result<Url, JsErrorBox> {
    match crate::check_url(url_str, allowlist) {
        Ok(u) => Ok(u),
        Err(crate::CheckUrlError::NotAllowed(msg)) => Err(JsErrorBox::generic(format!(
            "Network access to '{}' is not allowed",
            msg
        ))),
        Err(e) => Err(JsErrorBox::new("InvalidInput", e.to_string())),
    }
}

#[derive(serde::Deserialize)]
struct FetchOptions {
    method: Option<String>,
    headers: Option<HashMap<String, String>>,
    body: Option<String>,
}

#[derive(serde::Serialize)]
struct FetchResponse {
    status: u16,
    status_text: String,
    headers: HashMap<String, String>,
    body: String,
}

#[op2]
#[serde]
pub async fn op_fetch(
    state: std::rc::Rc<std::cell::RefCell<OpState>>,
    #[string] url_str: String,
    #[serde] options: Option<FetchOptions>,
) -> Result<FetchResponse, JsErrorBox> {
    let (client, allowlist) = {
        let state = state.borrow();
        (
            state.borrow::<Client>().clone(),
            state.borrow::<NetworkConfig>().allowlist.clone(),
        )
    };

    // Initial URL check to catch IP literals and early failures
    let url = check_url(&url_str, &allowlist)?;

    let method = if let Some(ref opts) = options {
        opts.method
            .as_deref()
            .unwrap_or("GET")
            .parse::<Method>()
            .map_err(|e| JsErrorBox::new("InvalidInput", e.to_string()))?
    } else {
        Method::GET
    };

    let mut builder = client.request(method, url);

    if let Some(opts) = options {
        if let Some(headers) = opts.headers {
            for (k, v) in headers {
                builder = builder.header(k, v);
            }
        }
        if let Some(body) = opts.body {
            builder = builder.body(body);
        }
    }

    let response = builder
        .send()
        .await
        .map_err(|e| JsErrorBox::generic(e.to_string()))?;

    let status = response.status().as_u16();
    let status_text = response
        .status()
        .canonical_reason()
        .unwrap_or("")
        .to_string();

    let mut headers: HashMap<String, String> = HashMap::new();
    for (k, v) in response.headers() {
        let key = k.to_string();
        let val = v.to_str().unwrap_or("").to_string();
        if key.eq_ignore_ascii_case("set-cookie") {
            // NOTE: This naive impl overwrites separate Set-Cookie headers because HashMap keys are unique.
            // Deno's `Headers` polyfill would need to handle this, but passing multiple values is complex with this simplistic interface.
            // For now, we follow the RFC 7230 §3.2.2 rule for other headers (joining with comma)
            // but since reqwest headers iter iterates multiple entries, we need to manually join.
            // Wait, reqwest's `iter()` yields all values.
            // We need to collect them.
            // Let's assume non-set-cookie headers can be joined.
            // But HashMap prevents multiple entries.
            // Correct approach: check if exists, append.
            use std::fmt::Write;
            if let Some(existing) = headers.get_mut(&key) {
                if !key.eq_ignore_ascii_case("set-cookie") {
                    write!(existing, ", {}", val).ok();
                }
            } else {
                headers.insert(key, val);
            }
        } else {
            use std::fmt::Write;
            if let Some(existing) = headers.get_mut(&key) {
                write!(existing, ", {}", val).ok();
            } else {
                headers.insert(key, val);
            }
        }
    }

    // Stream body with limit
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|e| JsErrorBox::generic(e.to_string()))?;
        if body.len() + chunk.len() > MAX_FETCH_BODY_BYTES {
            return Err(JsErrorBox::generic(format!(
                "Fetch response body exceeded limit of {} bytes",
                MAX_FETCH_BODY_BYTES
            )));
        }
        body.extend_from_slice(&chunk);
    }

    let body_str = String::from_utf8(body).map_err(|e| JsErrorBox::generic(e.to_string()))?;

    Ok(FetchResponse {
        status,
        status_text,
        headers,
        body: body_str,
    })
}

#[op2(fast)]
pub fn op_set_output(state: &mut OpState, #[string] json: String) -> Result<(), Error> {
    if let Some(max_output_size) = state
        .try_borrow::<OutputConfig>()
        .and_then(|cfg| cfg.max_output_size)
    {
        if json.len() > max_output_size {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "output size limit exceeded (max: {} bytes, got: {} bytes)",
                    max_output_size,
                    json.len()
                ),
            ));
        }
    }

    let value: serde_json::Value = serde_json::from_str(&json).map_err(|e| {
        // Only log metadata and the error message to avoid leaking raw sandbox output
        eprintln!(
            "Failed to parse sandbox output JSON: {e}. Output length: {} bytes.",
            json.len()
        );
        Error::new(ErrorKind::InvalidData, e)
    })?;

    state.put(OutputState { value });
    Ok(())
}

deno_core::extension!(sandbox_ext, ops = [op_set_output, op_fetch],);

#[cfg(unix)]
fn current_thread_cpu_clock_id() -> Result<libc::clockid_t, SandboxError> {
    let mut clock_id: libc::clockid_t = 0;
    // SAFETY: pthread_self returns the current thread handle, and clock_id is valid writable memory.
    let rc = unsafe { pthread_getcpuclockid(pthread_self(), &mut clock_id) };
    if rc != 0 {
        return Err(SandboxError::RuntimeError(format!(
            "failed to resolve current thread CPU clock: {}",
            std::io::Error::from_raw_os_error(rc)
        )));
    }
    Ok(clock_id)
}

#[cfg(unix)]
fn thread_cpu_time(clock_id: libc::clockid_t) -> Result<Duration, SandboxError> {
    let mut ts = std::mem::MaybeUninit::<timespec>::uninit();
    // SAFETY: ts points to valid writable memory; clock_id was obtained via pthread_getcpuclockid.
    let rc = unsafe { clock_gettime(clock_id, ts.as_mut_ptr()) };
    if rc != 0 {
        return Err(SandboxError::RuntimeError(format!(
            "failed to read thread CPU usage: {}",
            std::io::Error::last_os_error()
        )));
    }

    // SAFETY: rc == 0 means ts is fully initialized by clock_gettime.
    let ts = unsafe { ts.assume_init() };
    if ts.tv_sec < 0 || ts.tv_nsec < 0 {
        return Err(SandboxError::RuntimeError(
            "thread CPU clock returned negative timestamp".to_string(),
        ));
    }
    if ts.tv_nsec >= 1_000_000_000 {
        return Err(SandboxError::RuntimeError(format!(
            "thread CPU clock returned out-of-range nanoseconds: {}",
            ts.tv_nsec
        )));
    }
    Ok(Duration::from_secs(ts.tv_sec as u64)
        .saturating_add(Duration::from_nanos(ts.tv_nsec as u64)))
}

#[async_trait]
impl SandboxRuntime for DenoSandbox {
    async fn execute(
        &mut self,
        code: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, SandboxError> {
        // Defense-in-depth: require explicit hardening acknowledgment in release builds
        // because deno_core embedding must run inside an OS/container sandbox.
        if !cfg!(debug_assertions)
            && std::env::var("FLEXISUITE_DENO_HARDENED").ok().as_deref() != Some("1")
        {
            return Err(SandboxError::InitError(
                "Deno runtime requires hardened sandbox deployment. Set FLEXISUITE_DENO_HARDENED=1 only when seccomp/AppArmor, least-privilege user, and secret isolation are enforced.".to_string(),
            ));
        }

        #[cfg(not(unix))]
        {
            if !self.options.cpu_time_limit.is_zero() {
                return Err(SandboxError::InitError(
                    "Deno CPU time limiting is unsupported on this platform".to_string(),
                ));
            }
        }

        let exec_permit = self
            .exec_limiter
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| {
                SandboxError::InitError("Deno execution semaphore is closed".to_string())
            })?;

        let options = self.options.clone();
        let code = code.to_string();

        let result = tokio::task::spawn_blocking(move || {
            let _exec_permit = exec_permit;
            if options.memory_limit < MIN_DENO_HEAP_LIMIT {
                return Err(SandboxError::InitError(format!(
                    "memory_limit ({}) is below the minimum Deno/V8 heap limit ({})",
                    options.memory_limit, MIN_DENO_HEAP_LIMIT
                )));
            }

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

                let resolver = Arc::new(crate::AllowlistResolver::new(
                    options.permissions.network_allowlist.clone(),
                ));
                let client = Client::builder()
                    .dns_resolver(resolver)
                    .timeout(Duration::from_secs(30))
                    .build()
                    .map_err(|e| SandboxError::InitError(e.to_string()))?;

                {
                    let op_state_rc = js_runtime.op_state();
                    let mut op_state = op_state_rc.borrow_mut();
                    op_state.put(OutputConfig {
                        max_output_size: Some(options.max_output_size.unwrap_or(DEFAULT_MAX_OUTPUT_SIZE)),
                    });
                    op_state.put(NetworkConfig {
                        allowlist: options.permissions.network_allowlist.clone(),
                    });
                    op_state.put(client);
                }

                let isolate_handle = js_runtime.v8_isolate().thread_safe_handle();
                let wall_clock_limit = options.wall_clock_limit;
                let cpu_time_limit = options.cpu_time_limit;
                let cancelled = Arc::new(AtomicBool::new(false));
                let is_heap_oom = Arc::new(AtomicBool::new(false));
                let is_wall_timeout = Arc::new(AtomicBool::new(false));
                let is_cpu_timeout = Arc::new(AtomicBool::new(false));
                let is_cpu_clock_failed = Arc::new(AtomicBool::new(false));

                let near_heap_isolate = isolate_handle.clone();
                let near_heap_flag = is_heap_oom.clone();
                js_runtime.add_near_heap_limit_callback(move |current_limit, _initial_limit| {
                    near_heap_flag.store(true, Ordering::SeqCst);
                    near_heap_isolate.terminate_execution();
                    current_limit
                });

                let mut watchdogs = WatchdogGroup::new(cancelled.clone());
                let watchdog_cancel = cancelled.clone();
                let wall_timeout_flag = is_wall_timeout.clone();
                let wall_isolate_handle = isolate_handle.clone();
                watchdogs.spawn(std::thread::spawn(move || {
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
                }));

                #[cfg(unix)]
                {
                    let cpu_cancel = cancelled.clone();
                    let cpu_timeout_flag = is_cpu_timeout.clone();
                    let cpu_clock_failed_flag = is_cpu_clock_failed.clone();
                    let cpu_isolate_handle = isolate_handle.clone();
                    let cpu_clock_id = current_thread_cpu_clock_id()?;
                    let cpu_start = thread_cpu_time(cpu_clock_id)?;
                    watchdogs.spawn(std::thread::spawn(move || {
                        let sleep_slice = Duration::from_millis(10);
                        while !cpu_cancel.load(Ordering::SeqCst) {
                            match thread_cpu_time(cpu_clock_id) {
                                Ok(now) if now.saturating_sub(cpu_start) >= cpu_time_limit => {
                                    cpu_timeout_flag.store(true, Ordering::SeqCst);
                                    cpu_isolate_handle.terminate_execution();
                                    return;
                                }
                                Ok(_) => {}
                                Err(_) => {
                                    cpu_clock_failed_flag.store(true, Ordering::SeqCst);
                                    cpu_isolate_handle.terminate_execution();
                                    return;
                                }
                            }
                            if cpu_cancel.load(Ordering::SeqCst) {
                                return;
                            }
                            std::thread::sleep(sleep_slice);
                        }
                    }));
                }
                #[cfg(not(unix))]
                {
                    // CPU limiting is not supported on non-Unix platforms.
                    // Since we want production-grade robustness (Rule 1), we warn that it's disabled.
                    eprintln!("Warning: CPU time limiting is not supported on this platform.");
                }

                let input_json = serde_json::to_string(&input)
                    .map_err(|e| SandboxError::RuntimeError(format!("failed to serialize input: {e}")))?;
                let setup_code = format!(
                    r#"
                    globalThis.INPUT = {};
                    // Minimal polyfill for Headers
                    // Note: This is a subset implementation.
                    globalThis.Headers = class Headers {{
                        constructor(init) {{
                            this.map = new Map(Object.entries(init || {{}}));
                        }}
                        append(name, value) {{
                            const k = name.toLowerCase();
                            const v = String(value);
                            if (this.map.has(k)) {{
                                this.map.set(k, this.map.get(k) + ", " + v);
                            }} else {{
                                this.map.set(k, v);
                            }}
                        }}
                        delete(name) {{ this.map.delete(name.toLowerCase()); }}
                        get(name) {{ return this.map.get(name.toLowerCase()) || null; }}
                        has(name) {{ return this.map.has(name.toLowerCase()); }}
                        set(name, value) {{ this.map.set(name.toLowerCase(), String(value)); }}
                        forEach(callback) {{ this.map.forEach((v, k) => callback(v, k, this)); }}
                    }};
                    globalThis.fetch = async function(url, options) {{
                        // Convert Headers instance to plain object
                        if (options && options.headers instanceof Headers) {{
                            const plainHeaders = {{}};
                            options.headers.forEach((v, k) => {{
                                plainHeaders[k] = v;
                            }});
                            options = {{ ...options, headers: plainHeaders }};
                        }}
                        const response = await Deno.core.ops.op_fetch(url, options);
                        return {{
                            ok: response.status >= 200 && response.status < 300,
                            status: response.status,
                            statusText: response.status_text,
                            headers: new Headers(response.headers),
                            url: url,
                            redirected: false, // stub
                            type: 'basic', // stub
                            bodyUsed: false, // stub
                            clone: function() {{ return {{ ...this }}; }}, // naive clone stub
                            arrayBuffer: async () => {{
                                const encoder = new TextEncoder();
                                return encoder.encode(response.body).buffer;
                            }},
                            text: async () => response.body,
                            json: async () => JSON.parse(response.body),
                        }};
                    }};
                    "#,
                    input_json
                );
                js_runtime
                    .execute_script("setup", setup_code)
                    .map_err(|e| SandboxError::RuntimeError(e.to_string()))?;

                let wrapped_code = format!(
                    r#"
                    (async function() {{
                        try {{
                            const result = eval({});
                            const resolved = await Promise.resolve(result);
                            Deno.core.ops.op_set_output(JSON.stringify(resolved === undefined ? null : resolved));
                        }} catch (error) {{
                            const message = error instanceof Error ? error.message : String(error);
                            Deno.core.ops.op_set_output(JSON.stringify({{ "__sandbox_error__": message }}));
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
                    Err(err) => {
                        if is_heap_oom.load(Ordering::SeqCst) {
                            return Err(SandboxError::MemoryLimitExceeded);
                        }

                        if is_cpu_clock_failed.load(Ordering::SeqCst) {
                            return Err(SandboxError::RuntimeError(
                                "failed to enforce CPU limit due to thread CPU clock error".to_string(),
                            ));
                        }

                        if is_cpu_timeout.load(Ordering::SeqCst) {
                            return Err(SandboxError::CpuLimitExceeded);
                        }

                        if is_wall_timeout.load(Ordering::SeqCst) {
                            return Err(SandboxError::Timeout);
                        }

                        Err(err)
                    }
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
