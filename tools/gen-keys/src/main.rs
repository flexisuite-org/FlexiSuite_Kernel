use chrono::{TimeZone, Utc};
use clap::Parser;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::RngCore;
use rand::rngs::OsRng;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Parser)]
#[command(name = "gen-keys", about = "Generate trust root keys and JSON")]
struct Args {
    /// Output file path for manifest_trust_root.json.
    /// Priority: CLI arg > GEN_KEYS_OUTPUT_PATH > built-in default.
    #[arg(long, env = "GEN_KEYS_OUTPUT_PATH")]
    output: Option<PathBuf>,

    /// Optional manifest version override (default: UTC timestamp-based version).
    #[arg(long, env = "GEN_KEYS_VERSION")]
    version: Option<String>,

    /// Optional key id override (default: unique runtime-generated key id).
    #[arg(long, env = "GEN_KEYS_KID")]
    kid: Option<String>,

    /// Key validity window in days (default: 365).
    #[arg(long, env = "GEN_KEYS_VALIDITY_DAYS", default_value_t = 365)]
    validity_days: u64,
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
    let now_duration = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let now_secs = now_duration.as_secs();
    let now_dt = Utc
        .timestamp_opt(now_secs as i64, now_duration.subsec_nanos())
        .single()
        .expect("SystemTime::now() and UNIX_EPOCH must produce a valid UTC timestamp");
    let generated_at = now_dt.to_rfc3339();

    let validity_secs = args.validity_days.saturating_mul(24 * 60 * 60);
    let not_before = now_secs;
    let not_after = now_secs.saturating_add(validity_secs);
    let version = args
        .version
        .unwrap_or_else(|| format!("v{}", now_dt.format("%Y%m%dT%H%M%SZ")));

    let mut kid_suffix = [0_u8; 8];
    csprng.fill_bytes(&mut kid_suffix);
    let generated_kid = format!("store-key-{}-{}", now_secs, hex::encode(kid_suffix));
    let kid = args.kid.unwrap_or(generated_kid);

    println!("Public Key: {}", pub_hex);
    println!("Key ID: {}", kid);

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
    let output_path = match args.output {
        Some(path) => path,
        None => default_output_path()?,
    };

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

fn default_output_path() -> Result<PathBuf, std::io::Error> {
    let cwd = std::env::current_dir()?;
    Ok(cwd.join("ops/trust/manifest_trust_root.json"))
}
