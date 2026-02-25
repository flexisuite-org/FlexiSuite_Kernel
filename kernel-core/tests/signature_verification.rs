use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use kernel_core::supplychain::{
    KeyStatus, Manifest, TrustedKey, VerificationResult, verify_manifest,
};
use rand::rngs::OsRng;

#[test]
fn test_manifest_signature_verification_real_crypto() {
    let mut csprng = OsRng;
    let signing_key = SigningKey::generate(&mut csprng);
    let verifying_key: VerifyingKey = (&signing_key).into();
    let public_bytes = verifying_key.to_bytes();

    let digest = "sha256-real-crypto-test";
    let signature = signing_key.sign(digest.as_bytes());
    let signature_hex = hex::encode(signature.to_bytes());

    let manifest = Manifest {
        id: "pkg-crypto".to_string(),
        digest: digest.to_string(),
        signature: signature_hex,
        kid: "active-key".to_string(),
    };

    let trusted_key = TrustedKey {
        kid: "active-key".to_string(),
        status: KeyStatus::Active,
        retired_at: None,
        public_key: public_bytes.to_vec(),
    };

    let now = 100000;
    let result = verify_manifest(&manifest, &trusted_key, digest, now);

    assert!(matches!(result, VerificationResult::Ok));
}
