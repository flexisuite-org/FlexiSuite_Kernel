use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use tracing::{error, info, warn};
use sysinfo::System;

#[derive(Debug, Serialize, Deserialize)]
pub struct SloProfile {
    pub version: String,
    pub updated_at: String,
    pub region: RegionConfig,
    pub network: NetworkConfig,
    pub api_node: ApiNodeConfig,
    // Other fields are loaded but maybe not checked by api-node itself
    #[serde(flatten)]
    pub other: serde_yml::Value,
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
    let paths = ["ops/slo_profile.yaml", "../ops/slo_profile.yaml"];

    for path_str in paths {
        let path = Path::new(path_str);
        if path.exists() {
            info!("Loading SLO profile from {:?}", path);
            match File::open(path) {
                Ok(file) => {
                    let reader = BufReader::new(file);
                    match serde_yml::from_reader(reader) {
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
        warn!("Environment check failed: CPU count {} < required {}", cpu_count, profile.api_node.cpu_vcpu);
        compliant = false;
    } else {
        info!("Environment check passed: CPU count {} >= required {}", cpu_count, profile.api_node.cpu_vcpu);
    }

    // Check Memory
    // total_memory() returns bytes
    let total_mem_bytes = sys.total_memory();
    let total_mem_gb = total_mem_bytes / 1024 / 1024 / 1024;

    // Approximate check (allow some slack or exact?)
    // Usually memory reported is slightly less than physical due to kernel reservation.
    // 90% threshold seems reasonable.
    let required_gb = profile.api_node.memory_gb;
    if total_mem_gb < (required_gb * 90 / 100) {
         warn!("Environment check failed: Memory {} GB < required {} GB", total_mem_gb, required_gb);
         compliant = false;
    } else {
         info!("Environment check passed: Memory {} GB >= required {} GB", total_mem_gb, required_gb);
    }

    compliant
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_profile() {
        // Ensure we can load the profile from the repo
        let profile = load_profile();
        assert!(profile.is_some(), "Should load profile from ops/slo_profile.yaml or ../ops/slo_profile.yaml");
        let p = profile.unwrap();
        assert_eq!(p.version, "1");
    }
}
