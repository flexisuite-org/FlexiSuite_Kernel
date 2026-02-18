use kernel_core::auth::TenantId;
use kernel_core::diagnostics::{DiagnosticPolicy, is_diagnostics_enabled};

#[test]
fn test_diagnostics_opt_out_default() {
    assert!(
        !is_diagnostics_enabled(None),
        "In-memory check: Diagnostics MUST be opt-out by default"
    );
}

#[test]
fn test_diagnostics_policy_revocation() {
    let tenant_id = TenantId::new("tenant-diag").unwrap();

    let enabled = DiagnosticPolicy::new(tenant_id.clone(), true, None);
    assert!(is_diagnostics_enabled(Some(&enabled)));

    let revoked = DiagnosticPolicy::new(tenant_id, false, None);
    assert!(
        !is_diagnostics_enabled(Some(&revoked)),
        "In-memory revoked policy disables diagnostics"
    );
}
