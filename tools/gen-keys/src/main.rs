use clap::Parser;
use ed25519_dalek::{SigningKey, Signer, VerifyingKey};
use rand::rngs::OsRng;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "gen-keys", about = "Generate trust root keys and JSON")]
struct Args {
    /// Output file path for manifest_trust_root.json.
    /// Priority: CLI arg > GEN_KEYS_OUTPUT_PATH > built-in default.
    #[arg(long, env = "GEN_KEYS_OUTPUT_PATH")]
    output: Option<PathBuf>,
}

#[derive(Serialize)]
struct TrustRoot {
    version: String,
    generated_at: String,
    keys: Vec<Key>,
}

#[derive(Serialize)]
struct Key {
    kid: String,
    alg: String,
    public_key: String,
    status: String,
    not_before: u64,
    not_after: u64,
}

fn main() {
    let args = Args::parse();
    let mut csprng = OsRng;
    let signing_key: SigningKey = SigningKey::generate(&mut csprng);
    let verifying_key: VerifyingKey = signing_key.verifying_key();

    let pub_hex = hex::encode(verifying_key.to_bytes());
    let priv_hex = hex::encode(signing_key.to_bytes());

    println!("Private Key (Keep Safe): {}", priv_hex);
    println!("Public Key: {}", pub_hex);

    let trust_root = TrustRoot {
        version: "2026-02-15".to_string(),
        generated_at: "2026-02-15T00:00:00Z".to_string(),
        keys: vec![Key {
            kid: "store-key-2026-01".to_string(),
            alg: "Ed25519".to_string(),
            public_key: pub_hex,
            status: "active".to_string(),
            not_before: 1700000000,
            not_after: 1900000000,
        }],
    };

    let json = serde_json::to_string_pretty(&trust_root).unwrap();

    let output_path = args.output.unwrap_or_else(default_output_path);

    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).unwrap();
        }
    }

    fs::write(&output_path, json).unwrap();
    println!(
        "Wrote manifest_trust_root.json to {}",
        output_path.display()
    );
}

fn default_output_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../ops/trust/manifest_trust_root.json")
}
