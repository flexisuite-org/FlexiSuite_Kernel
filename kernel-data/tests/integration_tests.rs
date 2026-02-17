use kernel_core::auth::{KeyManager, TenantContext, TenantId, UserId};
use kernel_data::connection::{with_tenant_tx, TenantScoped, RawConnection};
use kernel_data::repository::TenantRepository;
use migration::MigratorTrait;
use sea_orm::{ActiveValue, ConnectionTrait, Database, DbBackend, Statement, TransactionTrait};
use testcontainers::{RunnableImage, clients};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;
use kernel_data::entities::entity_record;
use kernel_data::DataError;

const TEST_INTERNAL_SECRET: &str = "test_internal_secret_123";

#[tokio::test(flavor = "multi_thread")]
async fn test_tenant_isolation_rls() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    // 1. Connect
    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to connect to DB");

    // Verify Role Exists
    db.execute_unprepared("DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi') THEN CREATE ROLE flexi; END IF; END $$;").await.expect("Failed to create role flexi");

    // 2. Run Migrations
    migration::Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");

    // 3. Configure Internal Database Secret (GUC)
    db.execute_unprepared(&format!("ALTER ROLE postgres SET flexi.hmac_secret = '{}'", TEST_INTERNAL_SECRET))
        .await
        .expect("Failed to set flexi.hmac_secret for node");

    // 4. Reconnect to ensure all pool connections pick up the new GUC
    drop(db);
    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to reconnect to DB");

    // 5. Initialize Keys (HMAC & PASETO)
    KeyManager::rotate_keys(&db).await.expect("Failed to initialize keys");

    // 6. Test Logic
    let tenant_a = TenantContext::new(TenantId::new("tenant-a").unwrap(), Some(UserId::new("user-1").unwrap()));
    let tenant_b = TenantContext::new(TenantId::new("tenant-b").unwrap(), Some(UserId::new("user-2").unwrap()));
    
    
    let record_id = Uuid::now_v7().to_string();

    // A. Insert as Tenant A
    let record_id_a = record_id.clone();
    let token_a_1 = KeyManager::generate_tenant_token(&db, tenant_a.tenant_id().as_str()).await.expect("gen token A1");
    with_tenant_tx(&db, &tenant_a, &token_a_1, |repo| Box::pin(async move {
        insert_record(repo, record_id_a, "bar").await
    })).await.expect("Tenant A insert failed");

    // B. Read as Tenant A (Should succeed)
    let record_id_clone = record_id.clone();
    let token_a_2 = KeyManager::generate_tenant_token(&db, tenant_a.tenant_id().as_str()).await.expect("gen token A2");
    with_tenant_tx(&db, &tenant_a, &token_a_2, |repo| Box::pin(async move {
        let res = repo.get_entity(&record_id_clone).await?;
        assert!(res.is_some(), "Tenant A should see their own record");
        Ok(())
    })).await.expect("Tenant A read failed");

    // C. Read as Tenant B (Should fail/empty)
    let record_id_clone = record_id.clone();
    let token_b_1 = KeyManager::generate_tenant_token(&db, tenant_b.tenant_id().as_str()).await.expect("gen token B1");
    with_tenant_tx(&db, &tenant_b, &token_b_1, |repo| Box::pin(async move {
        let res = repo.get_entity(&record_id_clone).await?;
        assert!(res.is_none(), "Tenant B should NOT see Tenant A's record");
        Ok(())
    })).await.expect("Tenant B read failed");
    
    // D. Insert as Tenant B (Should succeed independent of A)
    // Note: ID reuse across tenants?
    // entity_record PK is (id, tenant_id). So same ID is allowed for different tenant.
    let record_id_b = record_id.clone();
    let token_b_2 = KeyManager::generate_tenant_token(&db, tenant_b.tenant_id().as_str()).await.expect("gen token B2");
    with_tenant_tx(&db, &tenant_b, &token_b_2, |repo| Box::pin(async move {
        insert_record(repo, record_id_b, "baz").await
    })).await.expect("Tenant B insert failed");
}

async fn insert_record(repo: &TenantScoped<RawConnection>, id: String, val: &str) -> Result<(), DataError> {
    let active_model = entity_record::ActiveModel {
        id: ActiveValue::Set(id),
        entity_type: ActiveValue::Set("test".to_string()),
        content: ActiveValue::Set(serde_json::json!({"foo": val})),
        ..Default::default()
    };
    repo.create_entity(active_model).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_migration_succeeds_without_flexi_role() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to connect to DB");

    // Ensure role flexi does NOT exist
    let rows = db
        .query_all(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT 1 FROM pg_roles WHERE rolname = 'flexi'",
            [],
        ))
        .await
        .expect("Failed to query roles");
    assert_eq!(rows.len(), 0, "Role flexi should not exist yet");

    // Run Migrations
    migration::Migrator::up(&db, None)
        .await
        .expect("Migration failed when role flexi is missing");

    // Verify schema exists
    let schema_exists = db
        .query_all(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT 1 FROM information_schema.schemata WHERE schema_name = 'flexi'",
            [],
        ))
        .await
        .expect("Failed to query schema");
    assert_eq!(schema_exists.len(), 1, "Schema flexi should exist");
}
