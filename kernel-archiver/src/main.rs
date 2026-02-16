use std::env;
use std::time::Duration;
use sea_orm::{Database, DatabaseConnection, EntityTrait, QueryFilter, ColumnTrait, Set, ActiveModelTrait, IntoActiveModel, QuerySelect};
use tokio::time;
use tracing::{info, error};
use kernel_data::entities::prelude::*;
use kernel_data::entities::{entity_history, audit_log};
use aws_config::meta::region::RegionProviderChain;
use aws_sdk_s3::{Client, config::Region};
use aws_sdk_s3::primitives::ByteStream;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use sha2::{Sha256, Digest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let s3_bucket = env::var("AUDIT_LOG_BUCKET").unwrap_or_else(|_| "audit-logs".to_string());
    let region_name = env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());

    // Connect to DB
    let db: DatabaseConnection = Database::connect(&database_url).await?;
    info!("Connected to database");

    // Init S3 Client
    let region_provider = RegionProviderChain::first_try(Region::new(region_name));
    let config = aws_config::from_env().region(region_provider).load().await;
    let s3_client = Client::new(&config);

    let interval_secs = env::var("ARCHIVER_INTERVAL").unwrap_or_else(|_| "60".to_string()).parse::<u64>().unwrap_or(60);
    let mut interval = time::interval(Duration::from_secs(interval_secs));

    loop {
        interval.tick().await;
        info!("Starting archive cycle...");

        if let Err(e) = archive_entity_history(&db, &s3_client, &s3_bucket).await {
            error!("Error archiving entity history: {}", e);
        }

        if let Err(e) = archive_audit_logs(&db, &s3_client, &s3_bucket).await {
            error!("Error archiving audit logs: {}", e);
        }

        info!("Archive cycle completed.");
    }
}

async fn archive_entity_history(db: &DatabaseConnection, s3: &Client, bucket: &str) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Find unarchived records
    // Limit to batch size to avoid memory issues
    let batch_size = 100;

    let histories = EntityHistory::find()
        .filter(entity_history::Column::ArchivedAt.is_null())
        .limit(batch_size)
        .all(db)
        .await?;

    if histories.is_empty() {
        return Ok(());
    }

    info!("Found {} entity history records to archive", histories.len());

    for history in histories {
        let json = serde_json::to_string(&history)?;
        let key = format!("entity-history/{}/{}/{}.json.gz",
            history.tenant_id,
            history.created_at.format("%Y/%m/%d"),
            history.id
        );

        // Compress
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(json.as_bytes())?;
        let compressed_data = encoder.finish()?;

        // Checksum
        let mut hasher = Sha256::new();
        hasher.update(&compressed_data);
        let checksum = format!("{:x}", hasher.finalize());

        // Upload to S3
        let body = ByteStream::from(compressed_data);

        let put_result = s3.put_object()
            .bucket(bucket)
            .key(&key)
            .body(body)
            .content_type("application/json")
            .content_encoding("gzip")
            .metadata("worm-compliant", "true")
            .metadata("sha256", &checksum)
            // .checksum_sha256(checksum) // SDK v0.36 doesn't fully support this cleanly yet
            .send()
            .await;

        match put_result {
            Ok(_) => {
                // Update archived_at
                let mut active: entity_history::ActiveModel = history.into_active_model();
                let id = active.id.clone().unwrap();
                active.archived_at = Set(Some(chrono::Utc::now().into()));
                active.update(db).await?;
                info!("Archived entity history {}", id);
            }
            Err(e) => {
                error!("Failed to upload to S3: {}. Skipping update.", e);
            }
        }
    }

    Ok(())
}

async fn archive_audit_logs(db: &DatabaseConnection, s3: &Client, bucket: &str) -> Result<(), Box<dyn std::error::Error>> {
    let batch_size = 100;

    let logs = AuditLog::find()
        .filter(audit_log::Column::ArchivedAt.is_null())
        .limit(batch_size)
        .all(db)
        .await?;

    if logs.is_empty() {
        return Ok(());
    }

    info!("Found {} audit logs to archive", logs.len());

    for log in logs {
        let json = serde_json::to_string(&log)?;
        let key = format!("audit-logs/{}/{}/{}.json.gz",
            log.tenant_id,
            log.created_at.format("%Y/%m/%d"),
            log.id
        );

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(json.as_bytes())?;
        let compressed_data = encoder.finish()?;

        // Checksum
        let mut hasher = Sha256::new();
        hasher.update(&compressed_data);
        let checksum = format!("{:x}", hasher.finalize());

        let body = ByteStream::from(compressed_data);

        let put_result = s3.put_object()
            .bucket(bucket)
            .key(&key)
            .body(body)
            .content_type("application/json")
            .content_encoding("gzip")
            .metadata("worm-compliant", "true")
            .metadata("sha256", &checksum)
            .send()
            .await;

        match put_result {
            Ok(_) => {
                let mut active: audit_log::ActiveModel = log.into_active_model();
                let id = active.id.clone().unwrap();
                active.archived_at = Set(Some(chrono::Utc::now().into()));
                active.update(db).await?;
                info!("Archived audit log {}", id);
            }
            Err(e) => {
                error!("Failed to upload to S3: {}. Skipping update.", e);
            }
        }
    }

    Ok(())
}
