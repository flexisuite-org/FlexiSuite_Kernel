fn main() {
    // Fail if release-like builds include development-only features.
    // We check the CARGO_CFG_DEBUG_ASSERTIONS variable.
    let release_like = std::env::var("CARGO_CFG_DEBUG_ASSERTIONS").unwrap_or_default() != "1";

    if release_like && std::env::var("CARGO_FEATURE_ENABLE_DEV_AUTH").is_ok() {
        panic!(
            "enable_dev_auth must not be enabled in release builds—it enables development-only auth tokens."
        );
    }
    if release_like && std::env::var("CARGO_FEATURE_TEST_UTILS").is_ok() {
        panic!("test-utils must not be enabled in release builds—it bypasses authentication.");
    }

    println!("cargo:rerun-if-env-changed=CARGO_CFG_DEBUG_ASSERTIONS");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_ENABLE_DEV_AUTH");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_TEST_UTILS");
}
