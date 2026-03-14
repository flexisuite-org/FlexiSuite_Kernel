fn main() {
    // Fail if release-like builds include development-only features.
    // PROFILE names are user-defined, so we also inspect DEBUG/OPT_LEVEL.
    let release_like = std::env::var("DEBUG")
        .map(|v| v == "false")
        .unwrap_or(false)
        || std::env::var("OPT_LEVEL")
            .map(|v| v != "0")
            .unwrap_or(false);

    if release_like && std::env::var("CARGO_FEATURE_ENABLE_DEV_AUTH").is_ok() {
        panic!(
            "enable_dev_auth must not be enabled in release builds—it enables development-only auth tokens."
        );
    }

    println!("cargo:rerun-if-env-changed=PROFILE");
    println!("cargo:rerun-if-env-changed=DEBUG");
    println!("cargo:rerun-if-env-changed=OPT_LEVEL");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_ENABLE_DEV_AUTH");
}
