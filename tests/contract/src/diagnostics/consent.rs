#![allow(dead_code)]
#![allow(unused_imports)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// --- Contract Definitions ---

#[derive(Debug, Clone)]
struct DiagnosticsPolicy {
    tenant_id: String,
    enabled: bool,
    updated_at: Instant,
    updated_by: String,
}

struct DiagnosticsService {
    policies: Mutex<HashMap<String, DiagnosticsPolicy>>,
}

impl DiagnosticsService {
    fn new() -> Self {
        Self {
            policies: Mutex::new(HashMap::new()),
        }
    }

    // REQ-DIAG-CONSENT: Default opt-out
    fn is_enabled(&self, tenant_id: &str) -> bool {
        let policies = self.policies.lock().unwrap();
        if let Some(policy) = policies.get(tenant_id) {
            policy.enabled
        } else {
            false // Default is false (opt-out)
        }
    }

    fn update_policy(&self, tenant_id: &str, enabled: bool, user_id: &str) {
        let mut policies = self.policies.lock().unwrap();
        policies.insert(
            tenant_id.to_string(),
            DiagnosticsPolicy {
                tenant_id: tenant_id.to_string(),
                enabled,
                updated_at: Instant::now(),
                updated_by: user_id.to_string(),
            },
        );
    }
}

#[tokio::test]
async fn test_diagnostics_opt_out_default() {
    let service = DiagnosticsService::new();
    // No policy set -> Default should be false
    assert!(!service.is_enabled("tenant-1"), "Diagnostics MUST be opt-out by default");
}

#[tokio::test]
async fn test_diagnostics_policy_revocation() {
    let service = DiagnosticsService::new();
    let tenant_id = "tenant-A";
    let admin_user = "admin-1";

    // 1. Enable
    service.update_policy(tenant_id, true, admin_user);
    assert!(service.is_enabled(tenant_id));

    // 2. Revoke (Disable)
    service.update_policy(tenant_id, false, admin_user);

    // REQ-DIAG-CONSENT: Revocation must be immediate (or within 5 min cache)
    // Here logic is immediate.
    assert!(!service.is_enabled(tenant_id), "Revocation MUST stop transmission immediately");
}
