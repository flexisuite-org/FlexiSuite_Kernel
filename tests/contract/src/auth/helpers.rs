use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, SecondsFormat, Utc};
use kernel_api::auth::init_auth_config_with_public_key_and_revoked_kids_and_legacy_mode;
use rusty_paseto::core::{Key, PasetoAsymmetricPrivateKey};
use rusty_paseto::prelude::*;
use std::sync::OnceLock;

// Global initializer for auth config
static AUTH_INIT: OnceLock<()> = OnceLock::new();
// Global key cache for stability during a single test run
static TEST_KEYS: OnceLock<(Vec<u8>, String)> = OnceLock::new();

pub fn setup() {
    let (_, pub_b64) = get_test_keypair();
    AUTH_INIT.get_or_init(|| {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        init_auth_config_with_public_key_and_revoked_kids_and_legacy_mode(
            &pub_b64,
            &["revoked"],
            false,
        )
        .expect("Auth initialization failed");
    });
}

fn get_test_keypair() -> (Vec<u8>, String) {
    let (priv_bytes, pub_b64) = TEST_KEYS.get_or_init(|| {
        use ed25519_dalek::{SigningKey, VerifyingKey};
        use rand::rngs::OsRng;

        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key: VerifyingKey = (&signing_key).into();

        // PASETO V4 Public expects a 64-byte private key (seed + public)
        let mut combined = [0u8; 64];
        combined[..32].copy_from_slice(&signing_key.to_bytes());
        combined[32..].copy_from_slice(verifying_key.as_bytes());

        let pub_b64 = URL_SAFE_NO_PAD.encode(verifying_key.as_bytes());

        (combined.to_vec(), pub_b64)
    });
    (priv_bytes.clone(), pub_b64.clone())
}

pub fn generate_token(valid: bool) -> String {
    generate_token_with_kid(valid, Some("active"))
}

pub fn generate_token_with_kid(valid: bool, kid: Option<&str>) -> String {
    let (combined_bytes, _) = get_test_keypair();
    let key_array: [u8; 64] = combined_bytes.try_into().unwrap();
    let key = Key::<64>::from(key_array);
    let private_key = PasetoAsymmetricPrivateKey::<V4, Public>::from(&key);

    let now = Utc::now();
    let exp = if valid {
        now + Duration::hours(1)
    } else {
        now - Duration::hours(1)
    };
    let nbf = now - Duration::minutes(5);

    // kernel-api uses DateTime::parse_from_rfc3339
    let exp_str = exp.to_rfc3339_opts(SecondsFormat::Secs, true);
    let nbf_str = nbf.to_rfc3339_opts(SecondsFormat::Secs, true);
    let iat_str = now.to_rfc3339_opts(SecondsFormat::Secs, true);

    let footer = kid.map(|k| serde_json::json!({ "kid": k }).to_string());
    let mut builder = PasetoBuilder::<V4, Public>::default();

    builder.set_claim(CustomClaim::try_from(("tenant_id", "tenant_001")).unwrap());
    builder.set_claim(CustomClaim::try_from(("user_id", "user_123")).unwrap());
    builder.set_claim(ExpirationClaim::try_from(exp_str.as_str()).unwrap());
    builder.set_claim(NotBeforeClaim::try_from(nbf_str.as_str()).unwrap());
    builder.set_claim(IssuedAtClaim::try_from(iat_str.as_str()).unwrap());
    if let Some(footer) = footer.as_ref() {
        builder.set_footer(Footer::from(footer.as_str()));
    }

    builder.build(&private_key).expect("Paseto build failed")
}
