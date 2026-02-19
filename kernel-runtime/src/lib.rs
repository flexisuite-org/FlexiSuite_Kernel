use async_trait::async_trait;
use reqwest::dns::{Name, Resolve, Resolving};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use url::Url;

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
    pub network_allowlist: Vec<String>,
}

pub(crate) fn check_url(url_str: &str, allowlist: &[String]) -> Result<Url, String> {
    let url = Url::parse(url_str).map_err(|e| e.to_string())?;
    let host = url
        .host_str()
        .ok_or_else(|| "URL must have a host".to_string())?;

    if !allowlist.iter().any(|allowed| allowed == host) {
        return Err(format!("Network access to '{}' is not allowed", host));
    }
    Ok(url)
}

#[derive(Clone)]
pub struct AllowlistResolver {
    allowlist: Arc<Vec<String>>,
}

impl AllowlistResolver {
    pub fn new(allowlist: Vec<String>) -> Self {
        Self {
            allowlist: Arc::new(allowlist),
        }
    }
}

impl Resolve for AllowlistResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let allowlist = self.allowlist.clone();
        let name_str = name.as_str().to_string();

        Box::pin(async move {
            // 1. Check allowlist (hostname)
            if !allowlist.iter().any(|allowed| allowed == &name_str) {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("Network access to '{}' is not allowed", name_str),
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }

            // 2. Resolve
            // Using port 0 as we only need IPs
            let addrs = tokio::net::lookup_host((name_str.as_str(), 0)).await?;

            // 3. Validate IPs
            let addrs: Vec<SocketAddr> = addrs.collect();
            if addrs.is_empty() {
                return Err(Box::new(std::io::Error::other("No IP addresses resolved")));
            }

            for addr in &addrs {
                if !is_safe_ip(&addr.ip(), &name_str, &allowlist) {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!(
                            "Resolved IP {} for host {} is not allowed (Private/Loopback IPs are blocked unless explicitly listed)",
                            addr.ip(),
                            name_str
                        ),
                    )));
                }
            }

            Ok(Box::new(addrs.into_iter()) as Box<dyn Iterator<Item = SocketAddr> + Send>)
        })
    }
}

fn is_safe_ip(ip: &IpAddr, host: &str, allowlist: &[String]) -> bool {
    // If the IP is globally routable, it's safe.
    if is_global(ip) {
        return true;
    }

    // If IP is private/loopback, only allow if:
    // 1. The hostname IS the IP literal (and it passed the allowlist check).
    // 2. The allowlist explicitly contains "localhost" and the host is "localhost".
    // 3. The allowlist explicitly contains the IP literal? (already covered by 1 if host matches).

    // Check if host string is exactly this IP
    if host == ip.to_string() {
        // If host is IP literal, check_url already verified it is in allowlist.
        // So we are good.
        return true;
    }

    // Special case for localhost
    if host == "localhost" && allowlist.contains(&"localhost".to_string()) && ip.is_loopback() {
        return true;
    }

    false
}

fn is_global(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => !ipv4.is_private() && !ipv4.is_loopback() && !ipv4.is_link_local(),
        IpAddr::V6(ipv6) => !ipv6.is_loopback() && ((ipv6.segments()[0] & 0xfe00) != 0xfc00),
    }
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
