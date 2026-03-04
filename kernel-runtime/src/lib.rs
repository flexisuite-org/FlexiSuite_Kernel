use async_trait::async_trait;
use reqwest::dns::{Name, Resolve, Resolving};
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use url::Url;

pub mod deno_runtime;
pub mod wasm_runtime;

pub const MAX_FETCH_BODY_BYTES: usize = 10 * 1024 * 1024; // 10MB

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

#[derive(Debug, Error)]
pub enum CheckUrlError {
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("URL must have a host")]
    NoHost,
    #[error("Network access to '{0}' is not allowed")]
    NotAllowed(String),
    #[error("Unsupported URL scheme: {0}")]
    UnsupportedScheme(String),
}

fn normalize_allowlist_host(host: &str) -> String {
    let trimmed = host.trim();
    if let Ok(url) = Url::parse(trimmed)
        && let Some(parsed_host) = url.host_str()
    {
        return parsed_host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
    }
    trimmed
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

#[derive(Clone, Debug)]
struct UrlPrefixRule {
    scheme: String,
    host: String,
    explicit_port: Option<u16>,
    path_prefix: String,
}

impl UrlPrefixRule {
    fn from_url(url: &Url) -> Result<Self, CheckUrlError> {
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err(CheckUrlError::UnsupportedScheme(url.scheme().to_string()));
        }

        let host = url.host_str().ok_or(CheckUrlError::NoHost)?;
        let path_prefix = if url.path().is_empty() {
            "/".to_string()
        } else {
            url.path().to_string()
        };

        Ok(Self {
            scheme: url.scheme().to_ascii_lowercase(),
            host: normalize_allowlist_host(host),
            explicit_port: url.port(),
            path_prefix,
        })
    }

    fn matches(&self, url: &Url) -> bool {
        if url.scheme().to_ascii_lowercase() != self.scheme {
            return false;
        }

        let Some(url_host) = url.host_str() else {
            return false;
        };
        if normalize_allowlist_host(url_host) != self.host {
            return false;
        }

        if let Some(port) = self.explicit_port
            && url.port_or_known_default() != Some(port)
        {
            return false;
        }

        let path = url.path();
        if self.path_prefix == "/" {
            return true;
        }
        if !path.starts_with(&self.path_prefix) {
            return false;
        }
        if path.len() == self.path_prefix.len() {
            return true;
        }
        self.path_prefix.ends_with('/')
            || path
                .as_bytes()
                .get(self.path_prefix.len())
                .is_some_and(|b| *b == b'/')
    }
}

#[derive(Clone, Debug)]
pub(crate) struct NormalizedAllowlist {
    domain_set: Arc<HashSet<String>>,
    url_prefix_rules: Arc<Vec<UrlPrefixRule>>,
    resolver_host_set: Arc<HashSet<String>>,
}

impl NormalizedAllowlist {
    pub(crate) fn new(allowlist: &[String]) -> Self {
        let mut domain_set = HashSet::new();
        let mut url_prefix_rules = Vec::new();
        let mut resolver_host_set = HashSet::new();

        for entry in allowlist {
            let trimmed = entry.trim();
            if let Ok(url) = Url::parse(trimmed) {
                match UrlPrefixRule::from_url(&url) {
                    Ok(rule) => {
                        resolver_host_set.insert(rule.host.clone());
                        url_prefix_rules.push(rule);
                        continue;
                    }
                    // Do not reinterpret unsupported URL schemes as plain hosts.
                    Err(CheckUrlError::UnsupportedScheme(_)) => continue,
                    // URL input without host should be ignored, not widened.
                    Err(CheckUrlError::NoHost) => continue,
                    Err(CheckUrlError::ParseError(_)) | Err(CheckUrlError::NotAllowed(_)) => {}
                }
            }

            let host = normalize_allowlist_host(trimmed);
            resolver_host_set.insert(host.clone());
            domain_set.insert(host);
        }

        Self {
            domain_set: Arc::new(domain_set),
            url_prefix_rules: Arc::new(url_prefix_rules),
            resolver_host_set: Arc::new(resolver_host_set),
        }
    }

    pub(crate) fn contains_host(&self, host: &str) -> bool {
        self.resolver_host_set
            .contains(&normalize_allowlist_host(host))
    }

    pub(crate) fn is_url_allowed(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        let normalized_host = normalize_allowlist_host(host);

        if self.domain_set.contains(&normalized_host) {
            return true;
        }

        self.url_prefix_rules.iter().any(|rule| rule.matches(url))
    }
}

pub(crate) fn check_url(
    url_str: &str,
    allowlist: &NormalizedAllowlist,
) -> Result<Url, CheckUrlError> {
    let url = Url::parse(url_str).map_err(|e| CheckUrlError::ParseError(e.to_string()))?;

    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(CheckUrlError::UnsupportedScheme(url.scheme().to_string()));
    }

    let host = url.host_str().ok_or(CheckUrlError::NoHost)?;

    if !allowlist.is_url_allowed(&url) {
        return Err(CheckUrlError::NotAllowed(host.to_string()));
    }
    Ok(url)
}

#[derive(Clone)]
pub struct AllowlistResolver {
    allowlist: NormalizedAllowlist,
}

impl AllowlistResolver {
    pub fn new(allowlist: &[String]) -> Self {
        Self {
            allowlist: NormalizedAllowlist::new(allowlist),
        }
    }
}

impl Resolve for AllowlistResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let allowlist = self.allowlist.clone();
        let name_str = name.as_str().to_string();

        Box::pin(async move {
            // 1. Check allowlist (hostname)
            // We use check_url logic but repurposed since name is just host
            if !allowlist.contains_host(&name_str) {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("Network access to '{}' is not allowed", name_str),
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }

            // 2. Resolve
            // Using port 0 as we only need IPs
            let addrs = tokio::time::timeout(
                Duration::from_secs(3),
                tokio::net::lookup_host((name_str.as_str(), 0)),
            )
            .await
            .map_err(|_| {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("DNS lookup timed out for '{}'", name_str),
                )) as Box<dyn std::error::Error + Send + Sync>
            })??;

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

fn is_safe_ip(ip: &IpAddr, host: &str, allowlist: &NormalizedAllowlist) -> bool {
    // If the IP is globally routable, it's safe.
    if is_global(ip) {
        return true;
    }

    // If IP is private/loopback, only allow if:
    // 1. The hostname IS the IP literal (and it passed the allowlist check).
    // 2. The allowlist explicitly contains "localhost" and the host is "localhost".
    // 3. The allowlist explicitly contains the IP literal? (already covered by 1 if host matches).

    // Check if host string is this IP literal (supports bracketed IPv6 host forms).
    let host_ip = host
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(parsed_host_ip) = host_ip.parse::<IpAddr>() {
        if parsed_host_ip == *ip {
            return true;
        }
    }

    // Special case for localhost
    if host == "localhost" && allowlist.contains_host("localhost") && ip.is_loopback() {
        return true;
    }

    false
}

fn is_global(ip: &IpAddr) -> bool {
    let ip = match ip {
        IpAddr::V6(ipv6) => {
            if let Some(ipv4) = ipv6.to_ipv4_mapped() {
                IpAddr::V4(ipv4)
            } else {
                *ip
            }
        }
        _ => *ip,
    };

    match ip {
        IpAddr::V4(ipv4) => {
            // Check private, loopback, and link-local using standard methods first
            if ipv4.is_private() || ipv4.is_loopback() || ipv4.is_link_local() {
                return false;
            }

            // Explicitly deny additional ranges:
            // 0.0.0.0/8 (Current network)
            if ipv4.octets()[0] == 0 {
                return false;
            }
            // 100.64.0.0/10 (Shared Address Space / CGNAT)
            if ipv4.octets()[0] == 100 && (ipv4.octets()[1] & 0xC0) == 0x40 {
                return false;
            }
            // 192.0.0.0/24 (IETF Protocol Assignments) - partially covered by is_private but explicit
            if ipv4.octets()[0] == 192 && ipv4.octets()[1] == 0 && ipv4.octets()[2] == 0 {
                return false;
            }
            // 192.0.2.0/24 (TEST-NET-1) - documentation
            if ipv4.octets()[0] == 192 && ipv4.octets()[1] == 0 && ipv4.octets()[2] == 2 {
                return false;
            }
            // 198.18.0.0/15 (Benchmarking)
            if ipv4.octets()[0] == 198 && (ipv4.octets()[1] & 0xFE) == 0x12 {
                return false;
            }
            // 198.51.100.0/24 (TEST-NET-2) - documentation
            if ipv4.octets()[0] == 198 && ipv4.octets()[1] == 51 && ipv4.octets()[2] == 100 {
                return false;
            }
            // 203.0.113.0/24 (TEST-NET-3) - documentation
            if ipv4.octets()[0] == 203 && ipv4.octets()[1] == 0 && ipv4.octets()[2] == 113 {
                return false;
            }
            // 224.0.0.0/4 (Multicast) - covered by is_multicast but ensure
            if ipv4.is_multicast() {
                return false;
            }
            // 240.0.0.0/4 (Reserved)
            if (ipv4.octets()[0] & 0xF0) == 0xF0 {
                return false;
            }
            // 255.255.255.255 (Broadcast) - covered by is_broadcast
            if ipv4.is_broadcast() {
                return false;
            }

            true
        }
        IpAddr::V6(ipv6) => {
            if ipv6.is_loopback() {
                return false;
            }
            // Unspecified ::
            if ipv6.is_unspecified() {
                return false;
            }
            // Unique Local fc00::/7
            if (ipv6.segments()[0] & 0xfe00) == 0xfc00 {
                return false;
            }
            // Link-local fe80::/10
            if (ipv6.segments()[0] & 0xffc0) == 0xfe80 {
                return false;
            }
            // Multicast ff00::/8
            if ipv6.is_multicast() {
                return false;
            }
            // Discard Prefix 100::/64
            if ipv6.segments()[0] == 0x0100 {
                return false;
            }
            // Documentation 2001:db8::/32
            if ipv6.segments()[0] == 0x2001 && ipv6.segments()[1] == 0x0db8 {
                return false;
            }
            // NAT64 Well-Known Prefix 64:ff9b::/96
            if ipv6.segments()[0] == 0x0064
                && ipv6.segments()[1] == 0xff9b
                && ipv6.segments()[2] == 0
                && ipv6.segments()[3] == 0
                && ipv6.segments()[4] == 0
                && ipv6.segments()[5] == 0
            {
                return false;
            }

            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_allowlist_host_extracts_host_from_url() {
        assert_eq!(
            normalize_allowlist_host("https://Example.COM/path"),
            "example.com"
        );
        assert_eq!(
            normalize_allowlist_host("http://127.0.0.1:8080"),
            "127.0.0.1"
        );
    }

    #[test]
    fn check_url_accepts_allowlist_entry_written_as_url() {
        let allowlist = NormalizedAllowlist::new(&["https://example.com".to_string()]);
        assert!(check_url("https://example.com/api", &allowlist).is_ok());
    }

    #[test]
    fn check_url_enforces_scheme_port_and_path_prefix_for_url_entries() {
        let allowlist = NormalizedAllowlist::new(&["https://example.com:8443/api".to_string()]);

        assert!(check_url("https://example.com:8443/api/v1", &allowlist).is_ok());
        assert!(matches!(
            check_url("http://example.com:8443/api/v1", &allowlist),
            Err(CheckUrlError::NotAllowed(_))
        ));
        assert!(matches!(
            check_url("https://example.com/api/v1", &allowlist),
            Err(CheckUrlError::NotAllowed(_))
        ));
        assert!(matches!(
            check_url("https://example.com:8443/apix", &allowlist),
            Err(CheckUrlError::NotAllowed(_))
        ));
    }

    #[test]
    fn allowlist_url_entries_are_usable_for_dns_host_checks() {
        let allowlist = NormalizedAllowlist::new(&["https://example.com:8443/api".to_string()]);
        assert!(allowlist.contains_host("example.com"));
    }

    #[test]
    fn unsupported_url_scheme_is_not_treated_as_domain_allow_entry() {
        let allowlist = NormalizedAllowlist::new(&["ftp://internal.example/path".to_string()]);
        assert!(!allowlist.contains_host("internal.example"));
        assert!(matches!(
            check_url("https://internal.example/api", &allowlist),
            Err(CheckUrlError::NotAllowed(_))
        ));
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
