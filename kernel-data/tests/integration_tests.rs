use kernel_core::auth::TenantContext;
use kernel_core::kernel;
use kernel_data::connection::{RawConnection, TenantScoped, with_tenant_tx};
use kernel_data::entities::entity_record;
use kernel_data::repository::TenantRepository;
use migration::MigratorTrait;
use sea_orm::{
    ActiveValue, ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement,
    TransactionTrait,
};
use std::sync::OnceLock;
use testcontainers::{RunnableImage, clients};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

const TEST_HMAC_SECRET: &str = "test_secret_for_integration_tests_shared";

type PostgresNode = testcontainers::Container<'static, Postgres>;

fn get_docker_client() -> &'static clients::Cli {
    static DOCKER: OnceLock<&'static clients::Cli> = OnceLock::new();
    DOCKER.get_or_init(|| Box::leak(Box::new(clients::Cli::default())))
}

async fn setup_test_db() -> (DatabaseConnection, PostgresNode) {
    let docker = get_docker_client();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to connect to DB");

    if let Err(e) = kernel_data::init_hmac_secret_for_test(TEST_HMAC_SECRET) {
        assert!(
            e.contains("already initialized"),
            "Unexpected error from init_hmac_secret: {}",
            e
        );
    }

    db.execute_unprepared("DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi') THEN CREATE ROLE flexi; END IF; END $$;")
        .await
        .expect("Failed to create role flexi");
    migration::Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");
    db.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT set_config('flexi.hmac_secret', $1, false)",
        [TEST_HMAC_SECRET.into()],
    ))
    .await
    .expect("Failed to set flexi.hmac_secret for session");

    drop(db);
    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to reconnect to DB");
    (db, node)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_tenant_isolation_rls() {
    let (db, _node) = setup_test_db().await;

    use kernel_core::auth::{TenantId, UserId};
    let tenant_a = TenantContext::new(
        TenantId::new("tenant-a").unwrap(),
        Some(UserId::new("user-1").unwrap()),
    );
    let tenant_b = TenantContext::new(
        TenantId::new("tenant-b").unwrap(),
        Some(UserId::new("user-2").unwrap()),
    );

    let record_id = Uuid::now_v7().to_string();

    // A. Insert as Tenant A
    let record_id_a = record_id.clone();
    with_tenant_tx(&db, &tenant_a, |repo| {
        Box::pin(async move { insert_record(repo, record_id_a, "bar").await })
    })
    .await
    .expect("Tenant A insert failed");

    // B. Read as Tenant A (Should succeed)
    let record_id_clone = record_id.clone();
    with_tenant_tx(&db, &tenant_a, |repo| {
        Box::pin(async move {
            let res = repo.get_entity(&record_id_clone).await?;
            assert!(res.is_some(), "Tenant A should see their own record");
            Ok(())
        })
    })
    .await
    .expect("Tenant A read failed");

    // C. Read as Tenant B (Should fail/empty)
    let record_id_clone = record_id.clone();
    with_tenant_tx(&db, &tenant_b, |repo| {
        Box::pin(async move {
            let res = repo.get_entity(&record_id_clone).await?;
            assert!(res.is_none(), "Tenant B should NOT see Tenant A's record");
            Ok(())
        })
    })
    .await
    .expect("Tenant B read failed");

    // D. Insert as Tenant B (Should succeed independent of A)
    let record_id_b = record_id.clone();
    with_tenant_tx(&db, &tenant_b, |repo| {
        Box::pin(async move { insert_record(repo, record_id_b, "baz").await })
    })
    .await
    .expect("Tenant B insert failed");

    // E. Verify uniqueness allows same ID for different tenants (if PK allows)
    // Our PK is (id, tenant_id), so this works.

    // f. Global Nonce Uniqueness Test
    let test_nonce = "once-only-nonce";
    let ts_1 = chrono::Utc::now().timestamp();
    let ts_2 = ts_1 + 10; // Different timestamp, same second window

    // First insert (Succeeds)
    // use sea_orm::{Statement, DbBackend}; // Removed duplicate import
    db.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "INSERT INTO flexi.flexi_nonce (nonce, created_at) VALUES ($1, to_timestamp($2))",
        [test_nonce.into(), ts_1.into()],
    ))
    .await
    .expect("First nonce insert should succeed");

    // Second insert with same nonce but different TS (Should fail via Trigger)
    let res = db
        .execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO flexi.flexi_nonce (nonce, created_at) VALUES ($1, to_timestamp($2))",
            [test_nonce.into(), ts_2.into()],
        ))
        .await;

    assert!(
        res.is_err(),
        "Second nonce insert with different TS should fail"
    );
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("Nonce already used"),
        "Error message should mention nonce usage: {}",
        err
    );
}

async fn insert_record(
    repo: &TenantScoped<RawConnection>,
    id: String,
    val: &str,
) -> kernel::Result<()> {
    let active_model = entity_record::ActiveModel {
        id: ActiveValue::Set(id),
        // tenant_id will be overwritten by repository correctly
        entity_type: ActiveValue::Set("test".to_string()),
        content: ActiveValue::Set(serde_json::json!({"foo": val})),
        ..Default::default()
    };
    repo.create_entity(active_model).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_delete_entity_contract() {
    let (db, _node) = setup_test_db().await;

    use kernel_core::auth::{TenantId, UserId};
    let tenant_a = TenantContext::new(
        TenantId::new("tenant-delete-a").unwrap(),
        Some(UserId::new("user-1").unwrap()),
    );
    let tenant_b = TenantContext::new(
        TenantId::new("tenant-delete-b").unwrap(),
        Some(UserId::new("user-2").unwrap()),
    );

    let record_id = Uuid::now_v7().to_string();
    let record_id_for_insert = record_id.clone();
    with_tenant_tx(&db, &tenant_a, |repo| {
        Box::pin(async move { insert_record(repo, record_id_for_insert, "before-delete").await })
    })
    .await
    .expect("Tenant A insert failed");

    let record_id_for_cross_delete = record_id.clone();
    let cross_delete_result = with_tenant_tx(&db, &tenant_b, |repo| {
        Box::pin(async move { repo.delete_entity(&record_id_for_cross_delete).await })
    })
    .await;
    assert!(
        matches!(
            cross_delete_result,
            Err(kernel::KernelError::ValidationError(ref msg)) if msg == "Entity not found"
        ),
        "Cross-tenant delete must be treated as not found"
    );

    let record_id_for_delete = record_id.clone();
    with_tenant_tx(&db, &tenant_a, |repo| {
        Box::pin(async move { repo.delete_entity(&record_id_for_delete).await })
    })
    .await
    .expect("Tenant A delete failed");

    let record_id_for_get = record_id.clone();
    with_tenant_tx(&db, &tenant_a, |repo| {
        Box::pin(async move {
            let entity = repo.get_entity(&record_id_for_get).await?;
            assert!(entity.is_none(), "Deleted entity should not exist");
            Ok(())
        })
    })
    .await
    .expect("Tenant A get after delete failed");

    let missing_id = Uuid::now_v7().to_string();
    let missing_delete_result = with_tenant_tx(&db, &tenant_a, |repo| {
        Box::pin(async move { repo.delete_entity(&missing_id).await })
    })
    .await;
    assert!(
        matches!(
            missing_delete_result,
            Err(kernel::KernelError::ValidationError(ref msg)) if msg == "Entity not found"
        ),
        "Deleting missing entity must return validation not found"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_migration_succeeds_without_flexi_role() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to connect to DB");

    // Ensure role flexi does NOT exist (should be default in fresh PG, but good to check)
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

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_authorize_fails_without_secret() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to connect to DB");

    // Create role so migration has full effect
    db.execute_unprepared("CREATE ROLE flexi").await.ok();

    migration::Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");

    // Do NOT set flexi.hmac_secret

    // Try to call authorize_tenant
    let now = chrono::Utc::now().timestamp();
    let token = format!("v2:kid:{}:nonce:tenant:sig", now);

    let res = db
        .execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT flexi.authorize_tenant($1)",
            [token.into()],
        ))
        .await;

    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("HMAC secret not set"),
        "Error should be about missing secret: {}",
        err
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_authorize_integrity_bypass_attempt() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to connect to DB");
    db.execute_unprepared("CREATE ROLE flexi").await.ok();
    migration::Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");

    // Use a transaction to ensure session state (GUCs) persists across calls
    let txn = db.begin().await.expect("Failed to begin transaction");

    // Set a secret
    txn.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT set_config('flexi.hmac_secret', 'secret', true)",
        [],
    ))
    .await
    .expect("Failed to set secret");

    // 1. Set current_tenant manually (Simulating attack)
    txn.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT set_config('flexi.current_tenant', 'attacker', true)",
        [],
    ))
    .await
    .expect("Failed to set tenant");

    // 2. Try to call authorized_tenant_id()
    let res = txn
        .query_one(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT flexi.authorized_tenant_id()",
            [],
        ))
        .await;

    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("Tenant context integrity check failed"),
        "Should catch integrity violation: {}",
        err
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_connection_rejects_colon_injection() {
    use kernel_core::auth::TenantId; // Import needed

    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432); // No await
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to connect to DB");

    // Initialize HMAC Secret for tests (using deterministic secret specific to tests)
    // We need this because authorization expects it
    let _ = kernel_data::init_hmac_secret_for_test(TEST_HMAC_SECRET);

    // We can verify TenantId prevents creation.
    // The previous implementation used unsafe transmute which is hard to simplify.
    // Since `TenantId::new` validates, we can just assert that.
    // The `connection.rs` check is defense in depth for IF an invalid ID exists.
    // To verify that check, we MUST bypass `TenantId` validation.

    // Safety: modifying internal string of TenantId.
    // TenantId is a tuple struct `pub struct TenantId(String);`
    // We can use transmute if we trust the layout is just String.
    let _valid = TenantId::new("valid").unwrap();
    // Safety: Use test-only method to bypass validation instead of unsafe transmute
    let invalid = TenantId::new_unchecked("invalid:id");

    // We also need a TenantContext
    let ctx = TenantContext::new(invalid, None);

    // Now call with_tenant_tx
    let res = with_tenant_tx(&db, &ctx, |_| Box::pin(async { Ok(()) })).await;

    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("tenant_id must not contain ':'"),
        "Should match our new error check: {}",
        err
    );
}
