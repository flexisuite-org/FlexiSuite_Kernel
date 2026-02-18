use kernel_core::auth::{KeyManager, TenantContext, TenantId, UserId};
use kernel_data::connection::{RawConnection, TenantScoped, with_tenant_tx};
use kernel_data::entities::{audit_log, entity_history, entity_record};
use kernel_data::repository::TenantRepository;
use migration::MigratorTrait;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, Database, EntityTrait, QueryFilter, QueryOrder,
};
use testcontainers::{RunnableImage, clients};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

const TEST_INTERNAL_SECRET: &str = "test_internal_secret_for_audit";

fn expected_actor_id(tenant_id: &TenantId, user_id: &UserId) -> String {
    let scoped = format!("{}:{}", tenant_id.as_str(), user_id.as_str());
    let digest = ring::digest::digest(&ring::digest::SHA256, scoped.as_bytes());
    format!("uidh:{}", hex::encode(digest.as_ref()))
}

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

    // Role
    db.execute_unprepared("DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi') THEN CREATE ROLE flexi; END IF; END $$;").await.expect("Failed to create role");

    // Migrations
    migration::Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");

    // Set Internal Secret
    db.execute_unprepared(&format!(
        "ALTER ROLE postgres SET flexi.hmac_secret = '{}'",
        TEST_INTERNAL_SECRET
    ))
    .await
    .expect("Failed to set secret");

    drop(db);
    let db = Database::connect(&connection_string)
        .await
        .expect("Reconnect failed");

    // Init Keys
    KeyManager::rotate_keys(&db)
        .await
        .expect("Failed to init keys");

    // Tenant Context
    let tenant_id = TenantId::new("audit-tenant").unwrap();
    let user_id = UserId::new("user-audit").unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), Some(user_id.clone()));

    // 1. Create Entity -> Should create history
    let entity_id = Uuid::now_v7().to_string();
    let entity_id_clone = entity_id.clone();
    let token_create = KeyManager::generate_tenant_token(&db, &tenant_id)
        .await
        .expect("Failed to gen token for create");

    with_tenant_tx(
        &db,
        &ctx,
        &token_create,
        |repo: &TenantScoped<RawConnection>| {
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
        },
    )
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
    assert_eq!(
        histories[0].created_by,
        expected_actor_id(&tenant_id, &user_id)
    );
    assert_eq!(histories[0].archived_at, None);

    // 2. Update Entity -> Should create history
    let entity_id_clone = entity_id.clone();
    let token_update = KeyManager::generate_tenant_token(&db, &tenant_id)
        .await
        .expect("Failed to gen token for update");
    with_tenant_tx(
        &db,
        &ctx,
        &token_update,
        |repo: &TenantScoped<RawConnection>| {
            Box::pin(async move {
                let active_model = entity_record::ActiveModel {
                    id: ActiveValue::Set(entity_id_clone),
                    content: ActiveValue::Set(serde_json::json!({"val": 2})),
                    ..Default::default()
                };
                repo.update_entity(active_model).await?;
                Ok(())
            })
        },
    )
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
    assert_eq!(
        histories[1].diff,
        serde_json::json!([{"op": "replace", "path": "/val", "value": 2}])
    );

    // 3. Log Audit
    let token_audit = KeyManager::generate_tenant_token(&db, &tenant_id)
        .await
        .expect("Failed to gen token for audit");
    with_tenant_tx(
        &db,
        &ctx,
        &token_audit,
        |repo: &TenantScoped<RawConnection>| {
            Box::pin(async move {
                repo.log_audit(
                    "test.action".to_string(),
                    "resource:1".to_string(),
                    serde_json::json!({"details": "foo"}),
                )
                .await?;
                Ok(())
            })
        },
    )
    .await
    .expect("Log audit failed");

    let logs = audit_log::Entity::find()
        .filter(audit_log::Column::TenantId.eq(tenant_id.to_string()))
        .all(&db)
        .await
        .expect("Failed to query audit logs");

    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].action, "test.action");
    assert_eq!(logs[0].actor_id, expected_actor_id(&tenant_id, &user_id));
    assert_eq!(logs[0].archived_at, None);

    // 4. Cross-tenant isolation checks
    let other_tenant_id = TenantId::new("audit-tenant-other").unwrap();
    let other_user_id = UserId::new("user-audit-other").unwrap();
    let other_ctx = TenantContext::new(other_tenant_id.clone(), Some(other_user_id.clone()));
    let other_token = KeyManager::generate_tenant_token(&db, &other_tenant_id)
        .await
        .expect("Failed to gen token other");

    let entity_id_clone = entity_id.clone();
    with_tenant_tx(
        &db,
        &other_ctx,
        &other_token,
        |repo: &TenantScoped<RawConnection>| {
            Box::pin(async move {
                let entity = repo.get_entity(&entity_id_clone).await?;
                assert!(entity.is_none(), "Cross-tenant entity should be invisible");
                Ok(())
            })
        },
    )
    .await
    .expect("Cross-tenant get_entity failed");

    let other_histories = entity_history::Entity::find()
        .filter(entity_history::Column::TenantId.eq(other_tenant_id.to_string()))
        .all(&db)
        .await
        .expect("Failed to query cross-tenant histories");
    assert!(
        other_histories.is_empty(),
        "Cross-tenant histories should be empty"
    );

    let other_logs = audit_log::Entity::find()
        .filter(audit_log::Column::TenantId.eq(other_tenant_id.to_string()))
        .all(&db)
        .await
        .expect("Failed to query cross-tenant audit logs");
    assert!(other_logs.is_empty(), "Cross-tenant logs should be empty");
}
