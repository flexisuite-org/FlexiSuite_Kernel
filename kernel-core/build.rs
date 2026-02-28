fn main() {
    // Hard fail if test-utils is enabled in release profile.
    // Production artifacts must never include test-only bypass paths.
    if std::env::var("PROFILE").unwrap_or_default() == "release"
        && std::env::var("CARGO_FEATURE_TEST_UTILS").is_ok()
    {
        panic!(
            "test-utils enabled in release profile. This is forbidden for production binaries as it bypasses security controls."
        );
    }
    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_TEST_UTILS");
}
