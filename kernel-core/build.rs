fn main() {
    // Hard fail if test-utils is enabled in release-like builds.
    // PROFILE names can be customized, so guard with DEBUG/OPT_LEVEL too.
    let release_like = std::env::var("DEBUG")
        .map(|v| v == "false")
        .unwrap_or(false)
        || std::env::var("OPT_LEVEL")
            .map(|v| v != "0")
            .unwrap_or(false);
    if release_like && std::env::var("CARGO_FEATURE_TEST_UTILS").is_ok() {
        panic!(
            "test-utils enabled in release profile. This is forbidden for production binaries as it bypasses security controls."
        );
    }
    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=DEBUG");
    println!("cargo:rerun-if-env-changed=OPT_LEVEL");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_TEST_UTILS");
}
