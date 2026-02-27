fn main() {
    // Fail if test-utils feature is enabled in release profile
    if std::env::var("PROFILE").unwrap_or_default() == "release"
        && std::env::var("CARGO_FEATURE_TEST_UTILS").is_ok()
    {
        panic!("test-utils must not be enabled in release builds—it bypasses authentication.");
    }
    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_TEST_UTILS");
}
