fn main() {
    let profile = std::env::var("PROFILE").unwrap_or_default();
    let test_utils_enabled = std::env::var("CARGO_FEATURE_TEST_UTILS").is_ok();

    if profile == "release" && test_utils_enabled {
        panic!("The 'test-utils' feature must not be enabled in release builds.");
    }
}
