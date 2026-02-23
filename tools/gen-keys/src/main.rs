use clap::Parser;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let mut csprng = OsRng;
    let signing_key: SigningKey = SigningKey::generate(&mut csprng);
    let verifying_key: VerifyingKey = signing_key.verifying_key();

    let pub_hex = hex::encode(verifying_key.to_bytes());
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let not_before = now_secs;
    let not_after = now_secs.saturating_add(Duration::from_secs(365 * 24 * 60 * 60).as_secs());
    let generated_at = format!("unix:{now_secs}");
    let version = format!("v{now_secs}");
    let kid = format!("store-key-{now_secs}");

    println!("Public Key: {}", pub_hex);

    let trust_root = TrustRoot {
        version,
        generated_at,
        keys: vec![Key {
            kid,
            alg: "Ed25519".to_string(),
            public_key: pub_hex,
            status: "active".to_string(),
            not_before,
            not_after,
        }],
    };

    let json = serde_json::to_string_pretty(&trust_root)?;

    let output_path = args.output.unwrap_or_else(default_output_path);

    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    fs::write(&output_path, json)?;
    println!(
        "Wrote manifest_trust_root.json to {}",
        output_path.display()
    );
    Ok(())
}

fn default_output_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../ops/trust/manifest_trust_root.json")
}
