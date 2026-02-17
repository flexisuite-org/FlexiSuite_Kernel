use std::env;
use std::io::Write;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use aws_config::meta::region::RegionProviderChain;
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::ObjectLockMode;
use aws_sdk_s3::{config::Region, Client};
use aws_smithy_types::DateTime as SmithyDateTime;
use base64::Engine as _;
use flate2::write::GzEncoder;
use flate2::Compression;
use kernel_core::auth::{TenantContext, TenantId};
use kernel_core::kernel::KernelError;
use kernel_data::{init_hmac_secret, with_tenant_tx, TenantRepository};
use sea_orm::{Database, DatabaseConnection};
use sha2::{Digest, Sha256};
use tokio::time;
use tracing::{error, info, warn};

#[derive(Clone)]
struct ObjectLockConfig {
    mode: ObjectLockMode,
    retain_until: SmithyDateTime,
}

#[derive(Clone)]
struct AppConfig {
    database_url: String,
    s3_bucket: String,
    region_name: String,
    interval_secs: u64,
    tenant_ids: Vec<TenantId>,
    object_lock: Option<ObjectLockConfig>,
}


#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config = load_config()?;
    init_hmac_secret().map_err(|e| anyhow!("failed to initialize HMAC secret: {e}"))?;

    let db: DatabaseConnection = Database::connect(&config.database_url)
        .await
        .context("failed to connect database")?;
    info!("Connected to database");

    let region_provider = RegionProviderChain::first_try(Region::new(config.region_name.clone()));
    let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(region_provider)
        .load()
        .await;
    let s3_client = Client::new(&aws_config);

    let mut interval = time::interval(Duration::from_secs(config.interval_secs));
    let mut shutdown_requested = false;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c(), if !shutdown_requested => {
                info!("Shutdown signal received. Current cycle (if any) will finish before exit.");
                shutdown_requested = true;
            }
            _ = interval.tick(), if !shutdown_requested => {
                info!("Starting archive cycle...");
                run_archive_cycle(&db, &s3_client, &config).await;
                info!("Archive cycle completed.");
            }
        }

        if shutdown_requested {
            break;
        }
    }

    info!("Archiver stopped gracefully.");
    Ok(())
}

fn load_config() -> Result<AppConfig> {
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL must be set")?;
    let s3_bucket = env::var("AUDIT_LOG_BUCKET").context("AUDIT_LOG_BUCKET must be set")?;

    let region_name = match env::var("AWS_REGION") {
        Ok(v) => v,
        Err(_) => {
            let default = "us-east-1".to_string();
            info!("AWS_REGION is not set, defaulting to {}", default);
            default
        }
    };

    let interval_secs = match env::var("ARCHIVER_INTERVAL") {
        Ok(v) => v
            .parse::<u64>()
            .context("ARCHIVER_INTERVAL must be an integer")?,
        Err(_) => {
            info!("ARCHIVER_INTERVAL is not set, defaulting to 60 seconds");
            60
        }
    };

    let tenant_ids = parse_tenant_ids(
        &env::var("ARCHIVER_TENANT_IDS")
            .context("ARCHIVER_TENANT_IDS must be set (comma-separated tenant IDs)")?,
    )?;

    let object_lock = parse_object_lock_config()?;
    if object_lock.is_none() {
        warn!("S3 Object Lock retention is disabled. Configure S3_OBJECT_LOCK_MODE and S3_OBJECT_LOCK_DAYS for WORM-grade retention.");
    }

    Ok(AppConfig {
        database_url,
        s3_bucket,
        region_name,
        interval_secs,
        tenant_ids,
        object_lock,
    })
}

fn parse_tenant_ids(raw: &str) -> Result<Vec<TenantId>> {
    let mut tenant_ids = Vec::new();
    for part in raw.split(',') {
        let tenant = part.trim();
        if tenant.is_empty() {
            continue;
        }
        tenant_ids.push(TenantId::new(tenant).map_err(|e| anyhow!(e))?);
    }

    if tenant_ids.is_empty() {
        return Err(anyhow!(
            "ARCHIVER_TENANT_IDS must contain at least one tenant"
        ));
    }

    Ok(tenant_ids)
}

fn parse_object_lock_config() -> Result<Option<ObjectLockConfig>> {
    let mode = match env::var("S3_OBJECT_LOCK_MODE") {
        Ok(v) => match v.to_ascii_uppercase().as_str() {
            "COMPLIANCE" => Some(ObjectLockMode::Compliance),
            "GOVERNANCE" => Some(ObjectLockMode::Governance),
            other => {
                return Err(anyhow!(
                    "S3_OBJECT_LOCK_MODE must be COMPLIANCE or GOVERNANCE, got {other}"
                ))
            }
        },
        Err(_) => None,
    };

    let Some(mode) = mode else {
        return Ok(None);
    };

    let days = match env::var("S3_OBJECT_LOCK_DAYS") {
        Ok(v) => v
            .parse::<i64>()
            .context("S3_OBJECT_LOCK_DAYS must be an integer")?,
        Err(_) => 365,
    };

    if days <= 0 {
        return Err(anyhow!("S3_OBJECT_LOCK_DAYS must be > 0"));
    }

    let retain_until =
        SmithyDateTime::from_secs((chrono::Utc::now() + chrono::Duration::days(days)).timestamp());

    Ok(Some(ObjectLockConfig { mode, retain_until }))
}

async fn run_archive_cycle(db: &DatabaseConnection, s3: &Client, config: &AppConfig) {
    for tenant_id in &config.tenant_ids {
        let ctx = TenantContext::new(tenant_id.clone(), None);
        let bucket = config.s3_bucket.clone();
        let lock_config = config.object_lock.clone();

        let result = with_tenant_tx(db, &ctx, |repo| {
            let s3 = s3.clone();
            Box::pin(async move {
                // Archive entity histories
                let histories = repo.find_unarchived_entity_histories(100).await?;
                if !histories.is_empty() {
                    info!("Found {} entity history records to archive", histories.len());
                    for history in histories {
                        let key = format!(
                            "entity-history/{}/{}/{}.json.gz",
                            sanitize_tenant_id_for_key(&history.tenant_id),
                            history.created_at.format("%Y/%m/%d"),
                            history.id
                        );

                        match s3_object_exists(&s3, &bucket, &key).await {
                            Ok(true) => {
                                if let Err(e) = repo.mark_entity_history_archived(history.id.clone()).await {
                                    error!("Failed to mark already-uploaded entity history archived: {}", e);
                                }
                                continue;
                            }
                            Ok(false) => {}
                            Err(e) => {
                                error!("head_object failed for entity history key {}: {}", key, e);
                                continue;
                            }
                        }

                        let json = serde_json::to_string(&history)
                            .map_err(|e| KernelError::DbError(e.to_string()))?;
                        let (payload, checksum) = build_gzip_payload(&json)
                            .map_err(|e| KernelError::DbError(e.to_string()))?;

                        match put_archive_object(&s3, &bucket, &key, payload, checksum, lock_config.as_ref()).await {
                            Ok(_) => {
                                if let Err(e) = repo.mark_entity_history_archived(history.id.clone()).await {
                                    error!("Failed to mark entity history archived after upload: {}", e);
                                }
                            }
                            Err(e) => {
                                error!("Failed to upload entity history {}: {}", key, e);
                            }
                        }
                    }
                }

                // Archive audit logs
                let logs = repo.find_unarchived_audit_logs(100).await?;
                if !logs.is_empty() {
                    info!("Found {} audit log records to archive", logs.len());
                    for log in logs {
                        let key = format!(
                            "audit-logs/{}/{}/{}.json.gz",
                            sanitize_tenant_id_for_key(&log.tenant_id),
                            log.created_at.format("%Y/%m/%d"),
                            log.id
                        );

                        match s3_object_exists(&s3, &bucket, &key).await {
                            Ok(true) => {
                                if let Err(e) = repo.mark_audit_log_archived(log.id.clone()).await {
                                    error!("Failed to mark already-uploaded audit log archived: {}", e);
                                }
                                continue;
                            }
                            Ok(false) => {}
                            Err(e) => {
                                error!("head_object failed for audit log key {}: {}", key, e);
                                continue;
                            }
                        }

                        let json = serde_json::to_string(&log)
                            .map_err(|e| KernelError::DbError(e.to_string()))?;
                        let (payload, checksum) = build_gzip_payload(&json)
                            .map_err(|e| KernelError::DbError(e.to_string()))?;

                        match put_archive_object(&s3, &bucket, &key, payload, checksum, lock_config.as_ref()).await {
                            Ok(_) => {
                                if let Err(e) = repo.mark_audit_log_archived(log.id.clone()).await {
                                    error!("Failed to mark audit log archived after upload: {}", e);
                                }
                            }
                            Err(e) => {
                                error!("Failed to upload audit log {}: {}", key, e);
                            }
                        }
                    }
                }

                Ok(())
            })
        })
        .await;

        if let Err(e) = result {
            error!("Tenant {} archive cycle failed: {}", tenant_id, e);
        }
    }
}


fn build_gzip_payload(json: &str) -> Result<(ByteStream, String)> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(json.as_bytes())?;
    let compressed_data = encoder.finish()?;

    let mut hasher = Sha256::new();
    hasher.update(&compressed_data);
    let checksum = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());

    Ok((ByteStream::from(compressed_data), checksum))
}

async fn put_archive_object(
    s3: &Client,
    bucket: &str,
    key: &str,
    body: ByteStream,
    checksum: String,
    object_lock: Option<&ObjectLockConfig>,
) -> Result<()> {
    let mut put_request = s3
        .put_object()
        .bucket(bucket)
        .key(key)
        .body(body)
        .content_type("application/gzip")
        .content_encoding("gzip")
        .checksum_sha256(checksum);

    if let Some(lock) = object_lock {
        put_request = put_request
            .object_lock_mode(lock.mode.clone())
            .object_lock_retain_until_date(lock.retain_until);
    }

    put_request.send().await?;
    Ok(())
}

async fn s3_object_exists(s3: &Client, bucket: &str, key: &str) -> Result<bool> {
    match s3.head_object().bucket(bucket).key(key).send().await {
        Ok(_) => Ok(true),
        Err(err) => {
            if let Some(service_err) = err.as_service_error() {
                let code = service_err.code();
                if code == Some("NotFound") || code == Some("404") {
                    return Ok(false);
                }
            }
            Err(anyhow!(err))
        }
    }
}

fn sanitize_tenant_id_for_key(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
