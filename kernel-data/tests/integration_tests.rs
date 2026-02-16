use kernel_core::auth::TenantContext;
use kernel_core::kernel;
use kernel_data::connection::{with_tenant_tx, TenantScoped, RawConnection};
use kernel_data::repository::TenantRepository;
use migration::MigratorTrait;
use sea_orm::{Database, ActiveValue, ConnectionTrait, TransactionTrait, Statement, DbBackend};
use testcontainers::{clients, RunnableImage};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;
use kernel_data::entities::entity_record;

#[tokio::test(flavor = "multi_thread")]
async fn test_tenant_isolation_rls() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    // 1. Connect
    let db = Database::connect(&connection_string).await.expect("Failed to connect to DB");

    // Initialize HMAC Secret for tests
    // Initialize HMAC Secret for tests (using deterministic secret specific to tests)
    if let Err(e) = kernel_data::init_hmac_secret_for_test("test_secret_for_integration_tests") {
        // Assert it's the expected "already initialized" error if it fails
        assert!(e.contains("already initialized"), "Unexpected error from init_hmac_secret: {}", e);
    }

    // Verify Role Exists (Grant statements in migration depend on it)
    db.execute_unprepared("DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi') THEN CREATE ROLE flexi; END IF; END $$;").await.expect("Failed to create role flexi");

    // 2. Run Migrations
    migration::Migrator::up(&db, None).await.expect("Failed to run migrations");

    // Mock Authorize Function (Simpler for test, or copy exact one)
    // NOTE: This mock intentionally skips signature verification and format validation
    // to simplify testing RLS isolation. It does NOT exercise the HMAC signing path fully.
    db.execute_unprepared(r#"
        DROP FUNCTION IF EXISTS flexi.authorize_tenant();
        DROP FUNCTION IF EXISTS flexi.authorize_tenant(text);
        CREATE OR REPLACE FUNCTION flexi.authorize_tenant(token_val text) RETURNS void AS $$
        DECLARE
            parts text[];
            tenant_id_val text;
            nonce_val text;
            ts bigint;
            secret text;
        BEGIN
            IF token_val IS NULL OR token_val = '' THEN
                RAISE EXCEPTION 'Missing or empty tenant token';
            END IF;
            
            parts := string_to_array(token_val, ':');
            -- v2:kid:ts:nonce:tenant_id:sig
            ts := parts[3]::bigint;
            nonce_val := parts[4];
            tenant_id_val := parts[5];
            
            BEGIN
                INSERT INTO flexi.flexi_nonce (nonce, created_at) 
                VALUES (nonce_val, to_timestamp(ts::double precision));
            EXCEPTION WHEN unique_violation THEN
                RAISE EXCEPTION 'Nonce already used: %', nonce_val;
            END;
            
            secret := current_setting('flexi.hmac_secret', true);
            PERFORM set_config('flexi.current_tenant', tenant_id_val, true);
            PERFORM set_config('flexi.ctx_sig', encode(hmac(tenant_id_val, secret, 'sha256'), 'hex'), true);
        END;
        $$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = flexi, pg_catalog, pg_temp;
    "#).await.unwrap();

    db.execute_unprepared(r#"
        CREATE OR REPLACE FUNCTION flexi.authorized_tenant_id() RETURNS text AS $$
        DECLARE
            tid text;
            sig text;
            secret text;
        BEGIN
            tid := current_setting('flexi.current_tenant', true);
            if tid IS NULL OR tid = '' THEN RETURN NULL; END IF;

            sig := current_setting('flexi.ctx_sig', true);
            secret := current_setting('flexi.hmac_secret', true);

            IF sig != encode(hmac(tid, secret, 'sha256'), 'hex') THEN
                RAISE EXCEPTION 'Tenant context integrity check failed';
            END IF;

            RETURN tid;
        END;
        $$ LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = flexi, pg_catalog, pg_temp;
    "#).await.unwrap();

    // Table
    db.execute_unprepared(r#"
        -- Table is created by migration, but we ensure RLS is on for the test
        ALTER TABLE flexi.entity_records DISABLE ROW LEVEL SECURITY;
        ALTER TABLE flexi.entity_records ENABLE ROW LEVEL SECURITY;
        DROP POLICY IF EXISTS tenant_isolation_policy ON flexi.entity_records;
        CREATE POLICY tenant_isolation_policy ON flexi.entity_records
            FOR ALL TO PUBLIC
            USING (tenant_id = flexi.authorized_tenant_id());
    "#).await.unwrap();
    // 3. Test Logic
    use kernel_core::auth::{TenantId, UserId};
    let tenant_a = TenantContext::new(TenantId::new("tenant-a").unwrap(), Some(UserId::new("user-1").unwrap()));
    let tenant_b = TenantContext::new(TenantId::new("tenant-b").unwrap(), Some(UserId::new("user-2").unwrap()));
    
    let record_id = Uuid::now_v7().to_string();

    // A. Insert as Tenant A
    let record_id_a = record_id.clone();
    with_tenant_tx(&db, &tenant_a, |repo| Box::pin(async move {
        insert_record(repo, record_id_a, "bar").await
    })).await.expect("Tenant A insert failed");

    // B. Read as Tenant A (Should succeed)
    let record_id_clone = record_id.clone();
    with_tenant_tx(&db, &tenant_a, |repo| Box::pin(async move {
        let res = repo.get_entity(&record_id_clone).await?;
        assert!(res.is_some(), "Tenant A should see their own record");
        Ok(())
    })).await.expect("Tenant A read failed");

    // C. Read as Tenant B (Should fail/empty)
    let record_id_clone = record_id.clone();
    with_tenant_tx(&db, &tenant_b, |repo| Box::pin(async move {
        let res = repo.get_entity(&record_id_clone).await?;
        assert!(res.is_none(), "Tenant B should NOT see Tenant A's record");
        Ok(())
    })).await.expect("Tenant B read failed");
    
    // D. Insert as Tenant B (Should succeed independent of A)
    let record_id_b = record_id.clone();
    with_tenant_tx(&db, &tenant_b, |repo| Box::pin(async move {
        insert_record(repo, record_id_b, "baz").await
    })).await.expect("Tenant B insert failed");
    
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
    )).await.expect("First nonce insert should succeed");
    
    // Second insert with same nonce but different TS (Should fail via Trigger)
    let res = db.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "INSERT INTO flexi.flexi_nonce (nonce, created_at) VALUES ($1, to_timestamp($2))",
        [test_nonce.into(), ts_2.into()],
    )).await;
    
    assert!(res.is_err(), "Second nonce insert with different TS should fail");
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Nonce already used"), "Error message should mention nonce usage: {}", err);
}

async fn insert_record(repo: &TenantScoped<RawConnection>, id: String, val: &str) -> kernel::Result<()> {
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
async fn test_migration_succeeds_without_flexi_role() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string).await.expect("Failed to connect to DB");

    // Ensure role flexi does NOT exist (should be default in fresh PG, but good to check)
    let rows = db.query_all(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT 1 FROM pg_roles WHERE rolname = 'flexi'",
        []
    )).await.expect("Failed to query roles");
    assert_eq!(rows.len(), 0, "Role flexi should not exist yet");

    // Run Migrations
    migration::Migrator::up(&db, None).await.expect("Migration failed when role flexi is missing");

    // Verify schema exists
    let schema_exists = db.query_all(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT 1 FROM information_schema.schemata WHERE schema_name = 'flexi'",
        []
    )).await.expect("Failed to query schema");
    assert_eq!(schema_exists.len(), 1, "Schema flexi should exist");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_authorize_fails_without_secret() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string).await.expect("Failed to connect to DB");

    // Create role so migration has full effect
    db.execute_unprepared("CREATE ROLE flexi").await.ok(); 

    migration::Migrator::up(&db, None).await.expect("Failed to run migrations");

    // Do NOT set flexi.hmac_secret

    // Try to call authorize_tenant
    let now = chrono::Utc::now().timestamp();
    let token = format!("v2:kid:{}:nonce:tenant:sig", now);

    let res = db.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT flexi.authorize_tenant($1)",
        [token.into()]
    )).await;
    
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("HMAC secret not set"), "Error should be about missing secret: {}", err);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_authorize_integrity_bypass_attempt() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string).await.expect("Failed to connect to DB");
    db.execute_unprepared("CREATE ROLE flexi").await.ok(); 
    migration::Migrator::up(&db, None).await.expect("Failed to run migrations");

    // Use a transaction to ensure session state (GUCs) persists across calls
    let txn = db.begin().await.expect("Failed to begin transaction");

    // Set a secret
    txn.execute(Statement::from_sql_and_values(
        DbBackend::Postgres, 
        "SELECT set_config('flexi.hmac_secret', 'secret', true)", 
        []
    )).await.expect("Failed to set secret");

    // 1. Set current_tenant manually (Simulating attack)
    txn.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT set_config('flexi.current_tenant', 'attacker', true)",
        []
    )).await.expect("Failed to set tenant");
    
    // 2. Try to call authorized_tenant_id()
    let res = txn.query_one(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT flexi.authorized_tenant_id()",
        []
    )).await;

    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Tenant context integrity check failed"), "Should catch integrity violation: {}", err);
}


#[tokio::test(flavor = "multi_thread")]
async fn test_connection_rejects_colon_injection() {
    use kernel_core::auth::TenantId; // Import needed

    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432); // No await
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string).await.expect("Failed to connect to DB");

    // Initialize HMAC Secret for tests (using deterministic secret specific to tests)
    // We need this because authorization expects it
    let _ = kernel_data::init_hmac_secret_for_test("test_secret_for_integration_tests_colons");

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
    assert!(err.contains("tenant_id must not contain ':'"), "Should match our new error check: {}", err);
}

// Removed test_hmac_secret_length_check as it's covered in security_tests.rs

