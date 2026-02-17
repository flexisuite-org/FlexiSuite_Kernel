use kernel_core::auth::{TenantContext, TenantId, UserId};
use kernel_data::connection::with_tenant_tx;
use kernel_data::entities::{audit_log, entity_history, entity_record};
use kernel_data::repository::TenantRepository;
use migration::MigratorTrait;
use sea_orm::{ActiveValue, ColumnTrait, ConnectionTrait, Database, EntityTrait, QueryFilter, QueryOrder};
use testcontainers::{RunnableImage, clients};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

const TEST_HMAC_SECRET: &str = "test_secret_for_audit_tests_shared";

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_audit_log_creation() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to connect to DB");

    // Init Secret
    if let Err(_) = kernel_data::init_hmac_secret_for_test(TEST_HMAC_SECRET) {
        // Ignore if already set
    }

    // Role
    db.execute_unprepared("DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi') THEN CREATE ROLE flexi; END IF; END $$;").await.expect("Failed to create role");

    // Migrations
    migration::Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");

    // Secret in DB
    let escaped_secret = TEST_HMAC_SECRET.replace('\'', "''");
    db.execute_unprepared(&format!(
        "ALTER ROLE postgres SET flexi.hmac_secret = '{}'",
        escaped_secret
    ))
    .await
    .expect("Failed to set secret");

    drop(db);
    let db = Database::connect(&connection_string)
        .await
        .expect("Reconnect failed");

    // Tenant Context
    let tenant_id = TenantId::new("audit-tenant").unwrap();
    let user_id = UserId::new("user-audit").unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), Some(user_id.clone()));

    // 1. Create Entity -> Should create history
    let entity_id = Uuid::now_v7().to_string();
    let entity_id_clone = entity_id.clone();

    with_tenant_tx(&db, &ctx, |repo| {
        Box::pin(async move {
            let active_model = entity_record::ActiveModel {
                id: ActiveValue::Set(entity_id_clone),
                entity_type: ActiveValue::Set("test-audit".to_string()),
                content: ActiveValue::Set(serde_json::json!({"val": 1})),
                ..Default::default()
            };
            repo.create_entity(active_model).await?;
            Ok(())
        })
    })
    .await
    .expect("Create entity failed");

    // Verify History
    let histories = entity_history::Entity::find()
        .filter(entity_history::Column::EntityId.eq(entity_id.clone()))
        .all(&db)
        .await
        .expect("Failed to query history");

    assert_eq!(histories.len(), 1, "Should have 1 history record");
    assert_eq!(histories[0].change_type, "CREATE");
    assert_eq!(histories[0].created_by, Some(user_id.to_string()));
    assert_eq!(histories[0].archived_at, None);

    // 2. Update Entity -> Should create history
    let entity_id_clone = entity_id.clone();
    with_tenant_tx(&db, &ctx, |repo| {
        Box::pin(async move {
            let active_model = entity_record::ActiveModel {
                id: ActiveValue::Set(entity_id_clone),
                content: ActiveValue::Set(serde_json::json!({"val": 2})),
                ..Default::default()
            };
            repo.update_entity(active_model).await?;
            Ok(())
        })
    })
    .await
    .expect("Update entity failed");

    let histories = entity_history::Entity::find()
        .filter(entity_history::Column::EntityId.eq(entity_id.clone()))
        .order_by_asc(entity_history::Column::Version)
        .all(&db)
        .await
        .expect("Failed to query history");

    assert_eq!(histories.len(), 2, "Should have 2 history records");
    assert_eq!(histories[1].change_type, "UPDATE");
    assert_eq!(histories[1].diff, serde_json::json!({"val": 2}));

    // 3. Log Audit
    with_tenant_tx(&db, &ctx, |repo| {
        Box::pin(async move {
            repo.log_audit(
                "test.action".to_string(),
                "resource:1".to_string(),
                serde_json::json!({"details": "foo"}),
            )
            .await?;
            Ok(())
        })
    })
    .await
    .expect("Log audit failed");

    let logs = audit_log::Entity::find()
        .filter(audit_log::Column::TenantId.eq(tenant_id.to_string()))
        .all(&db)
        .await
        .expect("Failed to query audit logs");

    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].action, "test.action");
    assert_eq!(logs[0].actor_id, user_id.to_string());
    assert_eq!(logs[0].archived_at, None);
}
