fn main() {
    // Fail if test-utils feature is enabled in release-like builds.
    // PROFILE names are user-defined, so we also inspect DEBUG/OPT_LEVEL.
    let release_like = std::env::var("DEBUG")
        .map(|v| v == "false")
        .unwrap_or(false)
        || std::env::var("OPT_LEVEL")
            .map(|v| v != "0")
            .unwrap_or(false);
    if release_like && std::env::var("CARGO_FEATURE_TEST_UTILS").is_ok() {
        panic!("test-utils must not be enabled in release builds—it bypasses authentication.");
    }
    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=DEBUG");
    println!("cargo:rerun-if-env-changed=OPT_LEVEL");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_TEST_UTILS");
}
