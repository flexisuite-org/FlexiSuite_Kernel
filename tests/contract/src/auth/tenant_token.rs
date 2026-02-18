use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use rusty_paseto::prelude::*;
use rusty_paseto::core::{Key, PasetoAsymmetricPrivateKey, Footer};
use std::sync::{Once, OnceLock};
use tower::ServiceExt; // for oneshot
use chrono::{Duration, Utc};
use kernel_api::auth::init_auth_config;
use crate::api::middleware_integration::setup_app;

static INIT: Once = Once::new();
static PRIVATE_KEY: OnceLock<Key<64>> = OnceLock::new();

// Valid Ed25519 Keypair
const PRIVATE_KEY_HEX: &str = "65364632d1a0ca52469c9697978f7e6b56f5da527bfbce884043a404e90f87fa84ae93b10f34f366beacdec5d7fc6977fa8721994de6982573668dcf6049c81f";
const PUBLIC_KEY_B64URL: &str = "hK6TsQ8082a-rN7F1_xpd_qHIZlN5pglc2aNz2BJyB8";

fn setup() {
    INIT.call_once(|| {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        unsafe {
            std::env::set_var("FLEXI_PASETO_V4_PUBLIC_KEY_B64URL", PUBLIC_KEY_B64URL);
        }
        let _ = init_auth_config();
    });
}

fn get_private_key() -> PasetoAsymmetricPrivateKey<'static, V4, Public> {
    let key = PRIVATE_KEY.get_or_init(|| {
        let key_bytes = hex::decode(PRIVATE_KEY_HEX).unwrap();
        let key_array: [u8; 64] = key_bytes.try_into().unwrap();
        Key::<64>::from(key_array)
    });
    PasetoAsymmetricPrivateKey::<V4, Public>::from(key)
}

fn generate_token(kid: Option<&str>, valid: bool) -> String {
    let private_key = get_private_key();

    let now = Utc::now();
    let exp = if valid { now + Duration::hours(1) } else { now - Duration::hours(1) };
    let nbf = now - Duration::minutes(5);
    let exp_str = exp.to_rfc3339();
    let nbf_str = nbf.to_rfc3339();

    // Declare strings before builder so they outlive builder
    let footer_string;

    let mut builder = PasetoBuilder::<V4, Public>::default();

    builder.set_claim(SubjectClaim::from("user_123"));
    builder.set_claim(ExpirationClaim::try_from(exp_str.as_str()).unwrap());
    builder.set_claim(NotBeforeClaim::try_from(nbf_str.as_str()).unwrap());
    builder.set_claim(CustomClaim::try_from(("tenant_id", "tenant_001")).unwrap());

    if let Some(kid_val) = kid {
         footer_string = format!(r#"{{"kid":"{}"}}"#, kid_val);
         builder.set_footer(Footer::from(footer_string.as_str()));
    }

    builder.build(&private_key).unwrap()
}

#[tokio::test]
async fn test_tenant_token_v2_kid_required() {
    setup();
    let app = setup_app().await;

    // Case 1: Token with KID (should pass)
    let token_with_kid = generate_token(Some("key-1"), true);
    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Authorization", format!("Bearer {}", token_with_kid))
        .header("Idempotency-Key", "test-key-1")
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();

    assert_eq!(res.status(), StatusCode::CREATED, "Valid token with KID should be accepted");

    // Case 2: Token WITHOUT KID (should fail)
    let token_no_kid = generate_token(None, true);
    let req = Request::builder()
        .uri("/test")
        .method("POST")
        .header("Authorization", format!("Bearer {}", token_no_kid))
        .header("Idempotency-Key", "test-key-2")
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();

    // Strict assertion
    assert!(
        res.status() == StatusCode::UNAUTHORIZED || res.status() == StatusCode::FORBIDDEN,
        "Token without KID must be rejected (got {})", res.status()
    );
}
