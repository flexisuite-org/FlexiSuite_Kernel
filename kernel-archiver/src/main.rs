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
use futures::future::BoxFuture;
use kernel_core::auth::{KeyManager, TenantContext, TenantId};
use kernel_data::connection::{RawConnection, TenantScoped};
use kernel_data::entities::{audit_log, entity_history};
use kernel_data::{init_hmac_secret, with_tenant_tx, DataError};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Database, DatabaseConnection, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Set,
};
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

trait ArchivableModel: serde::Serialize {
    const KEY_PREFIX: &'static str;
    const LABEL: &'static str;

    fn tenant_id(&self) -> &str;
    fn id(&self) -> &str;
    fn date_path(&self) -> String;
}

impl ArchivableModel for entity_history::Model {
    const KEY_PREFIX: &'static str = "entity-history";
    const LABEL: &'static str = "entity history";

    fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn date_path(&self) -> String {
        self.created_at.format("%Y/%m/%d").to_string()
    }
}

impl ArchivableModel for audit_log::Model {
    const KEY_PREFIX: &'static str = "audit-logs";
    const LABEL: &'static str = "audit log";

    fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn date_path(&self) -> String {
        self.created_at.format("%Y/%m/%d").to_string()
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

        // Archiver runs as system, uses its own keys to sign token
        let token = match KeyManager::generate_tenant_token(db, tenant_id.as_str()).await {
             Ok(t) => t,
             Err(e) => {
                 error!("Failed to generate token for tenant {}: {}", tenant_id, e);
                 continue;
             }
        };

        let result = with_tenant_tx(db, &ctx, &token, |repo: &TenantScoped<RawConnection>| {
            let s3 = s3.clone();
            Box::pin(async move {
                archive_entity_history(repo.txn(), &s3, &bucket, lock_config.as_ref())
                    .await
                    .map_err(|e| DataError::DbError(sea_orm::DbErr::Custom(e.to_string())))?;
                archive_audit_logs(repo.txn(), &s3, &bucket, lock_config.as_ref())
                    .await
                    .map_err(|e| DataError::DbError(sea_orm::DbErr::Custom(e.to_string())))?;
                Ok(())
            })
        })
        .await;

        if let Err(e) = result {
            error!("Tenant {} archive cycle failed: {}", tenant_id, e);
        }
    }
}

async fn archive_entity_history(
    db: &DatabaseTransaction,
    s3: &Client,
    bucket: &str,
    object_lock: Option<&ObjectLockConfig>,
) -> Result<()> {
    let histories = entity_history::Entity::find()
        .filter(entity_history::Column::ArchivedAt.is_null())
        .order_by_asc(entity_history::Column::CreatedAt)
        .limit(100)
        .all(db)
        .await?;

    if histories.is_empty() {
        return Ok(());
    }
    archive_batch(db, s3, bucket, object_lock, histories, |db, history| {
        Box::pin(mark_entity_history_archived(db, history))
    })
    .await
}

async fn archive_audit_logs(
    db: &DatabaseTransaction,
    s3: &Client,
    bucket: &str,
    object_lock: Option<&ObjectLockConfig>,
) -> Result<()> {
    let logs = audit_log::Entity::find()
        .filter(audit_log::Column::ArchivedAt.is_null())
        .order_by_asc(audit_log::Column::CreatedAt)
        .limit(100)
        .all(db)
        .await?;

    if logs.is_empty() {
        return Ok(());
    }
    archive_batch(db, s3, bucket, object_lock, logs, |db, log| {
        Box::pin(mark_audit_log_archived(db, log))
    })
    .await
}

async fn archive_batch<M, F>(
    db: &DatabaseTransaction,
    s3: &Client,
    bucket: &str,
    object_lock: Option<&ObjectLockConfig>,
    items: Vec<M>,
    mark_archived: F,
) -> Result<()>
where
    M: ArchivableModel,
    F: for<'a> Fn(&'a DatabaseTransaction, M) -> BoxFuture<'a, Result<()>>,
{
    info!("Found {} {} records to archive", items.len(), M::LABEL);

    for item in items {
        let key = format!(
            "{}/{}/{}/{}.json.gz",
            M::KEY_PREFIX,
            sanitize_tenant_id_for_key(item.tenant_id()),
            item.date_path(),
            item.id()
        );

        match s3_object_exists(s3, bucket, &key).await {
            Ok(true) => {
                if let Err(e) = mark_archived(db, item).await {
                    error!(
                        "Failed to mark already-uploaded {} archived: {}",
                        M::LABEL,
                        e
                    );
                }
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                error!("head_object failed for {} key {}: {}", M::LABEL, key, e);
                continue;
            }
        }

        let json = serde_json::to_string(&item)?;
        let (payload, checksum) = build_gzip_payload(&json)?;

        match put_archive_object(s3, bucket, &key, payload, checksum, object_lock).await {
            Ok(_) => {
                if let Err(e) = mark_archived(db, item).await {
                    error!("Failed to mark {} archived after upload: {}", M::LABEL, e);
                }
            }
            Err(e) => {
                error!("Failed to upload {} {}: {}", M::LABEL, key, e);
            }
        }
    }

    Ok(())
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

async fn mark_entity_history_archived(
    db: &DatabaseTransaction,
    history: entity_history::Model,
) -> Result<()> {
    let mut active: entity_history::ActiveModel = history.into_active_model();
    let id = match active.id.clone() {
        sea_orm::ActiveValue::Set(v) | sea_orm::ActiveValue::Unchanged(v) => v,
        sea_orm::ActiveValue::NotSet => "<unknown>".to_string(),
    };
    active.archived_at = Set(Some(chrono::Utc::now().into()));
    active
        .update(db)
        .await
        .with_context(|| format!("failed to update archived_at for entity history {}", id))?;
    info!("Archived entity history {}", id);
    Ok(())
}

async fn mark_audit_log_archived(db: &DatabaseTransaction, log: audit_log::Model) -> Result<()> {
    let mut active: audit_log::ActiveModel = log.into_active_model();
    let id = match active.id.clone() {
        sea_orm::ActiveValue::Set(v) | sea_orm::ActiveValue::Unchanged(v) => v,
        sea_orm::ActiveValue::NotSet => "<unknown>".to_string(),
    };
    active.archived_at = Set(Some(chrono::Utc::now().into()));
    active
        .update(db)
        .await
        .with_context(|| format!("failed to update archived_at for audit log {}", id))?;
    info!("Archived audit log {}", id);
    Ok(())
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
