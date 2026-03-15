fn main() {
    // Hard fail if development-only features are enabled in release-like builds.
    // PROFILE names can be customized, so guard with DEBUG/OPT_LEVEL instead of
    // relying on CARGO_CFG_DEBUG_ASSERTIONS for build-script execution.
    let release_like = std::env::var("DEBUG")
        .map(|v| v == "false")
        .unwrap_or(false)
        || std::env::var("OPT_LEVEL")
            .map(|v| v != "0")
            .unwrap_or(false);
    if release_like && std::env::var("CARGO_FEATURE_ENABLE_DEV_AUTH").is_ok() {
        panic!(
            "enable_dev_auth must not be enabled in release builds—it enables development-only auth headers."
        );
    }
    if release_like && std::env::var("CARGO_FEATURE_TEST_UTILS").is_ok() {
        panic!("test-utils must not be enabled in release builds—it bypasses authentication.");
    }
    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=DEBUG");
    println!("cargo:rerun-if-env-changed=OPT_LEVEL");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_ENABLE_DEV_AUTH");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_TEST_UTILS");
}
