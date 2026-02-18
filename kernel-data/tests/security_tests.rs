use kernel_data::connection::init_hmac_secret_for_test;

// Mock database connection for testing with_tenant_tx is difficult without a real DB.
// Instead, we will test the logic that we can isolate.
// However, the validation logic is inside `with_tenant_tx` which requires a DbConnection.
// We can use the integration tests for that.
//
// For `init_hmac_secret`, we can test it directly, but strict implementation of OnceLock might make it hard to test multiple times in one process if not careful.
// `init_hmac_secret_for_test` allows us to set it.

#[test]
fn test_hmac_secret_length_validation() {
    // This test assumes HMAC_SECRET has NOT been initialized yet in this test process runner.
    // Rust tests run in parallel threads by default, so we need to be careful.
    // We will try to set a SHORT secret and expect an error.

    // Note: If other tests ran before this and set the secret, this test might fail with "already initialized".
    // To reliably test this, we should probably put it in its own integration test file or run it first.
    // OR we can rely on the fact that `init_hmac_secret_from_string` checks length BEFORE checking `HMAC_SECRET.set`.

    let res = init_hmac_secret_for_test("short");
    assert_eq!(
        res.unwrap_err(),
        "FLEXI_HMAC_SECRET must be at least 32 bytes"
    );
}

// For TenantID validation:
// We need to bypass TenantId constructor to create an invalid one (with ':')
// and pass it to logic.
// But `with_tenant_tx` needs a DB connection.
// We can test this in `integration_tests.rs` effectively.
