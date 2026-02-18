use std::env;
use std::io::Write;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use aws_config::meta::region::RegionProviderChain;
use aws_sdk_s3::operation::head_object::HeadObjectError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::ObjectLockMode;
use aws_sdk_s3::{config::Region, Client};
use aws_smithy_types::DateTime as SmithyDateTime;
use base64::Engine as _;
use flate2::write::GzEncoder;
use flate2::Compression;
use kernel_core::auth::{TenantContext, TenantId};
use kernel_data::entities::{audit_log, entity_history};
use kernel_data::{init_hmac_secret, with_tenant_tx, TenantRepository};
use sea_orm::{Database, DatabaseConnection};
use sha2::{Digest, Sha256};
use tokio::time;
use tracing::{error, info, warn};

#[derive(Clone)]
struct ObjectLockConfig {
    mode: ObjectLockMode,
    retain_days: i64,
}

#[derive(Clone)]
struct AppConfig {
    database_url: String,
    s3_bucket: String,
    region_name: String,
    interval_secs: u64,
    batch_size: u64,
    tenant_ids: Vec<TenantId>,
    object_lock: Option<ObjectLockConfig>,
}

struct MarkPlan {
    entity_history_ids: Vec<String>,
    audit_log_ids: Vec<String>,
}

trait ArchivableItem {
    fn archive_id(&self) -> &str;
    fn archive_tenant_id(&self) -> &str;
    fn archive_created_at(&self) -> &chrono::DateTime<chrono::FixedOffset>;

    fn archive_key(&self, key_prefix: &str) -> String {
        format!(
            "{}/{}/{}/{}.json.gz",
            key_prefix,
            sanitize_tenant_id_for_key(self.archive_tenant_id()),
            self.archive_created_at().format("%Y/%m/%d"),
            sanitize_archive_id_for_key(self.archive_id())
        )
    }
}

impl ArchivableItem for entity_history::Model {
    fn archive_id(&self) -> &str {
        &self.id
    }

    fn archive_tenant_id(&self) -> &str {
        &self.tenant_id
    }

    fn archive_created_at(&self) -> &chrono::DateTime<chrono::FixedOffset> {
        &self.created_at
    }
}

impl ArchivableItem for audit_log::Model {
    fn archive_id(&self) -> &str {
        &self.id
    }

    fn archive_tenant_id(&self) -> &str {
        &self.tenant_id
    }

    fn archive_created_at(&self) -> &chrono::DateTime<chrono::FixedOffset> {
        &self.created_at
    }
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
                match run_archive_cycle(&db, &s3_client, &config).await {
                    Ok(()) => info!("Archive cycle completed."),
                    Err(e) => error!("Archive cycle failed: {}", e),
                }
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

    let batch_size = match env::var("ARCHIVER_BATCH_SIZE") {
        Ok(v) => {
            let parsed = v
                .parse::<u64>()
                .context("ARCHIVER_BATCH_SIZE must be an integer")?;
            if parsed == 0 {
                return Err(anyhow!("ARCHIVER_BATCH_SIZE must be > 0"));
            }
            parsed
        }
        Err(_) => {
            info!("ARCHIVER_BATCH_SIZE is not set, defaulting to 100");
            100
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
        batch_size,
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

    Ok(Some(ObjectLockConfig { mode, retain_days: days }))
}

async fn run_archive_cycle(db: &DatabaseConnection, s3: &Client, config: &AppConfig) -> Result<()> {
    for tenant_id in &config.tenant_ids {
        let ctx = TenantContext::new(tenant_id.clone(), None);
        let bucket = config.s3_bucket.clone();
        let lock_config = config.object_lock.clone();
        let batch_size = config.batch_size;

        let fetched = with_tenant_tx(db, &ctx, |repo| {
            Box::pin(async move {
                let histories = repo
                    .find_unarchived_entity_histories(batch_size)
                    .await?;
                let logs = repo.find_unarchived_audit_logs(batch_size).await?;
                Ok((histories, logs))
            })
        })
        .await;

        let (histories, logs) = match fetched {
            Ok(data) => data,
            Err(e) => {
                error!(
                    "Tenant {} failed to fetch unarchived records: {}",
                    tenant_id, e
                );
                continue;
            }
        };

        let tenant_id = ctx.tenant_id().to_string();
        let mark_plan = MarkPlan {
            entity_history_ids: archive_items(
                "entity history",
                "entity-history",
                histories,
                s3,
                &bucket,
                lock_config.as_ref(),
                &tenant_id,
            )
            .await?,
            audit_log_ids: archive_items(
                "audit log",
                "audit-logs",
                logs,
                s3,
                &bucket,
                lock_config.as_ref(),
                &tenant_id,
            )
            .await?,
        };

        if mark_plan.entity_history_ids.is_empty() && mark_plan.audit_log_ids.is_empty() {
            continue;
        }

        let mark_result = with_tenant_tx(db, &ctx, move |repo| {
            Box::pin(async move {
                if !mark_plan.entity_history_ids.is_empty() {
                    repo.mark_entity_histories_archived(mark_plan.entity_history_ids)
                        .await?;
                }
                if !mark_plan.audit_log_ids.is_empty() {
                    repo.mark_audit_logs_archived(mark_plan.audit_log_ids)
                        .await?;
                }
                Ok(())
            })
        })
        .await;

        if let Err(e) = mark_result {
            error!(
                "Tenant {} failed to mark archived records: {}",
                tenant_id, e
            );
        }
    }

    Ok(())
}

async fn archive_items<T>(
    record_kind: &str,
    key_prefix: &str,
    items: Vec<T>,
    s3: &Client,
    bucket: &str,
    lock_config: Option<&ObjectLockConfig>,
    expected_tenant_id: &str,
) -> Result<Vec<String>>
where
    T: ArchivableItem + serde::Serialize,
{
    if !items.is_empty() {
        info!("Found {} {} records to archive", items.len(), record_kind);
    }

    let mut archived_ids = Vec::new();
    for item in items {
        let id = item.archive_id().to_string();
        let item_tenant = item.archive_tenant_id();

        if item_tenant != expected_tenant_id {
            return Err(anyhow!(
                "Tenant mismatch while archiving item {} (kind: {}). Expected tenant_id={}, found tenant_id={}",
                id, record_kind, expected_tenant_id, item_tenant
            ));
        }

        let key = item.archive_key(key_prefix);

        match s3_object_exists(s3, bucket, &key).await {
            Ok(true) => {
                archived_ids.push(id);
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                error!("head_object failed for {} key {}: {}", record_kind, key, e);
                continue;
            }
        }

        let json = match serde_json::to_string(&item) {
            Ok(json) => json,
            Err(e) => {
                error!(
                    "Failed to serialize {} {} for archiving: {}",
                    record_kind, id, e
                );
                continue;
            }
        };

        let (payload, checksum) = match build_gzip_payload(&json) {
            Ok(payload) => payload,
            Err(e) => {
                error!("Failed to gzip {} {}: {}", record_kind, id, e);
                continue;
            }
        };

        match put_archive_object(s3, bucket, &key, payload, checksum, lock_config).await {
            Ok(_) => archived_ids.push(id),
            Err(e) => error!("Failed to upload {} {}: {}", record_kind, key, e),
        }
    }

    Ok(archived_ids)
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
        .checksum_sha256(checksum);

    if let Some(lock) = object_lock {
        // Compute retain_until at upload time so each object gets a fresh
        // WORM timestamp, even if the archiver has been running for hours.
        let retain_until = SmithyDateTime::from_secs(
            (chrono::Utc::now() + chrono::Duration::days(lock.retain_days)).timestamp(),
        );
        put_request = put_request
            .object_lock_mode(lock.mode.clone())
            .object_lock_retain_until_date(retain_until);
    }

    put_request.send().await?;
    Ok(())
}

async fn s3_object_exists(s3: &Client, bucket: &str, key: &str) -> Result<bool> {
    match s3.head_object().bucket(bucket).key(key).send().await {
        Ok(_) => Ok(true),
        Err(err) => {
            if matches!(err.as_service_error(), Some(HeadObjectError::NotFound(_))) {
                return Ok(false);
            }
            Err(anyhow!(err))
        }
    }
}

fn sanitize_tenant_id_for_key(raw: &str) -> String {
    sanitize_key_segment(raw)
}

fn sanitize_archive_id_for_key(raw: &str) -> String {
    sanitize_key_segment(raw)
}

fn sanitize_key_segment(raw: &str) -> String {
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
