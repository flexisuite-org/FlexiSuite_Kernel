use chrono::{TimeZone, Utc};
use clap::Parser;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand::RngCore;
use rand::rngs::OsRng;
use serde::Serialize;
use std::fs;
use std::io::{self, Write};
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

    /// Output path for the generated 32-byte Ed25519 private key.
    /// Use "-" to write raw bytes to stdout.
    #[arg(long, env = "GEN_KEYS_PRIVATE_KEY_OUTPUT")]
    private_key_output: PathBuf,
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
    let private_key_to_stdout = args.private_key_output.as_os_str() == "-";
    let mut csprng = OsRng;
    let signing_key: SigningKey = SigningKey::generate(&mut csprng);
    let private_key_bytes = signing_key.to_bytes();
    let verifying_key: VerifyingKey = signing_key.verifying_key();

    let pub_hex = hex::encode(verifying_key.to_bytes());
    let now_duration = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let now_secs = now_duration.as_secs();
    let subsec_nanos = now_duration.subsec_nanos();
    let now_dt = Utc
        .timestamp_opt(now_secs as i64, subsec_nanos)
        .single()
        .ok_or_else(|| {
            io::Error::other(format!(
                "SystemTime::now() and UNIX_EPOCH must produce a valid UTC timestamp (now_secs={now_secs}, subsec_nanos={subsec_nanos})"
            ))
        })?;
    let generated_at = now_dt.to_rfc3339();

    let validity_secs = args
        .validity_days
        .checked_mul(24 * 60 * 60)
        .ok_or_else(|| io::Error::other("validity_days is too large (overflow)"))?;
    let not_before = now_secs;
    let not_after = now_secs
        .checked_add(validity_secs)
        .ok_or_else(|| io::Error::other("Expiration timestamp overflow"))?;
    let version = args
        .version
        .unwrap_or_else(|| format!("v{}", now_dt.format("%Y%m%dT%H%M%SZ")));

    let mut kid_suffix = [0_u8; 8];
    csprng.fill_bytes(&mut kid_suffix);
    let generated_kid = format!("store-key-{}-{}", now_secs, hex::encode(kid_suffix));
    let kid = args.kid.unwrap_or(generated_kid);

    log_info(private_key_to_stdout, format_args!("Public Key: {}", pub_hex));
    log_info(private_key_to_stdout, format_args!("Key ID: {}", kid));

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

    write_private_key(
        &args.private_key_output,
        private_key_bytes.as_slice(),
        private_key_to_stdout,
    )?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&output_path, json)?;
    log_info(
        private_key_to_stdout,
        format_args!("Wrote manifest_trust_root.json to {}", output_path.display()),
    );
    Ok(())
}

fn default_output_path() -> Result<PathBuf, std::io::Error> {
    let cwd = std::env::current_dir()?;
    Ok(cwd.join("ops/trust/manifest_trust_root.json"))
}

fn write_private_key(
    path: &PathBuf,
    private_key_bytes: &[u8],
    private_key_to_stdout: bool,
) -> Result<(), io::Error> {
    if path.as_os_str() == "-" {
        let mut stdout = io::stdout();
        stdout.write_all(private_key_bytes)?;
        stdout.flush()?;
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(private_key_bytes)?;
        file.flush()?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }

    #[cfg(not(unix))]
    {
        fs::write(path, private_key_bytes)?;
    }

    log_info(
        private_key_to_stdout,
        format_args!("Wrote private key bytes to {}", path.display()),
    );
    Ok(())
}

fn log_info(to_stderr: bool, args: std::fmt::Arguments<'_>) {
    if to_stderr {
        eprintln!("{args}");
    } else {
        println!("{args}");
    }
}
