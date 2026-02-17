use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;
use sysinfo::System;
use tracing::{error, info, warn};

#[derive(Debug, Serialize, Deserialize)]
pub struct SloProfile {
    pub version: String,
    pub updated_at: String,
    pub region: RegionConfig,
    pub network: NetworkConfig,
    pub api_node: ApiNodeConfig,
    // Other fields are loaded but maybe not checked by api-node itself
    #[serde(flatten)]
    pub other: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegionConfig {
    pub mode: String,
    pub intra_region_only: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub client_server_rtt_ms_excluded: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiNodeConfig {
    pub cpu_vcpu: usize, // sysinfo returns usize for cpu count
    pub memory_gb: u64,  // sysinfo uses u64 for memory
    pub nic_gbps: u32,
}

pub fn load_profile() -> Option<SloProfile> {
    let override_path = std::env::var("SLO_PROFILE_PATH").ok().map(PathBuf::from);
    load_profile_with_path(override_path.as_deref())
}

pub fn load_profile_with_path(path_override: Option<&Path>) -> Option<SloProfile> {
    let mut paths: Vec<PathBuf> = Vec::new();

    if let Some(path) = path_override {
        paths.push(path.to_path_buf());
    }

    paths.push(PathBuf::from("ops/slo_profile.yaml"));
    paths.push(PathBuf::from("../ops/slo_profile.yaml"));

    for path in paths {
        if path.exists() {
            info!("Loading SLO profile from {:?}", path);
            match File::open(&path) {
                Ok(file) => {
                    let reader = BufReader::new(file);
                    match serde_yaml::from_reader(reader) {
                        Ok(profile) => return Some(profile),
                        Err(e) => {
                            error!("Failed to parse SLO profile: {}", e);
                            return None;
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to open SLO profile: {}", e);
                    return None;
                }
            }
        }
    }

    warn!("SLO profile not found in checked paths");
    None
}

pub fn check_environment(profile: &SloProfile) -> bool {
    let mut compliant = true;
    let mut sys = System::new();

    // We only need CPUs and Memory
    sys.refresh_cpu();
    sys.refresh_memory();

    // Check CPU
    // sys.cpus() returns a list of CPUs. The length is the count.
    // Or physical_core_count()? profile says "cpu_vcpu", usually means logical cores (threads).
    let cpu_count = sys.cpus().len();
    if cpu_count < profile.api_node.cpu_vcpu {
        warn!(
            "Environment check failed: CPU count {} < required {}",
            cpu_count, profile.api_node.cpu_vcpu
        );
        compliant = false;
    } else {
        info!(
            "Environment check passed: CPU count {} >= required {}",
            cpu_count, profile.api_node.cpu_vcpu
        );
    }

    // Check Memory
    // total_memory() returns bytes
    let total_mem_bytes = sys.total_memory();
    let gib_in_bytes = 1024_f64.powi(3);
    let total_mem_gib = total_mem_bytes as f64 / gib_in_bytes;

    // Approximate check (allow some slack or exact?)
    // Usually memory reported is slightly less than physical due to kernel reservation.
    // 90% threshold seems reasonable.
    let required_gb = profile.api_node.memory_gb;
    let required_threshold_bytes = required_gb as f64 * 0.9 * gib_in_bytes;
    if (total_mem_bytes as f64) < required_threshold_bytes {
        warn!(
            "Environment check failed: Memory {:.2} GiB < required {:.2} GiB (90% of {} GiB)",
            total_mem_gib,
            required_threshold_bytes / gib_in_bytes,
            required_gb
        );
        compliant = false;
    } else {
        info!(
            "Environment check passed: Memory {:.2} GiB >= required {:.2} GiB (90% of {} GiB)",
            total_mem_gib,
            required_threshold_bytes / gib_in_bytes,
            required_gb
        );
    }

    compliant
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_load_profile() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after UNIX_EPOCH")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("slo_profile_test_{}.yaml", unique));
        let yaml = r#"
version: "1"
updated_at: "2026-01-01T00:00:00Z"
region:
  mode: "single"
  intra_region_only: true
network:
  client_server_rtt_ms_excluded: false
api_node:
  cpu_vcpu: 1
  memory_gb: 1
  nic_gbps: 1
"#;

        fs::write(&path, yaml).expect("should write temp profile file");
        let profile = load_profile_with_path(Some(path.as_path()));
        fs::remove_file(&path).expect("should remove temp profile file");

        assert!(
            profile.is_some(),
            "Should load profile from explicit temp path"
        );
        let p = profile.expect("profile should be parsed");
        assert_eq!(p.version, "1");
    }
}
