use kernel_core::auth::TenantContext;
use kernel_core::kernel;
use kernel_data::connection::{with_tenant_tx, TenantScoped, RawConnection};
use kernel_data::repository::TenantRepository;
use migration::MigratorTrait;
use sea_orm::{Database, ActiveValue, ConnectionTrait};
use testcontainers::{clients, RunnableImage};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;
use kernel_data::entities::entity_record;

#[tokio::test]
async fn test_tenant_isolation_rls() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    // 1. Connect
    let db = Database::connect(&connection_string).await.expect("Failed to connect to DB");

    // Initialize HMAC Secret for tests
    unsafe { std::env::set_var("FLEXI_HMAC_SECRET", "test_secret_for_integration_tests"); }
    let _ = kernel_data::init_hmac_secret();

    // 2. Run Migrations
    migration::Migrator::up(&db, None).await.expect("Failed to run migrations");

    // Mock Authorize Function (Simpler for test, or copy exact one)
    db.execute_unprepared(r#"
        DROP FUNCTION IF EXISTS flexi.authorize_tenant();
        CREATE OR REPLACE FUNCTION flexi.authorize_tenant() RETURNS void AS $$
        DECLARE
            token_val text;
            parts text[];
            tenant_id_val text;
            nonce_val text;
            ts bigint;
        BEGIN
            token_val := current_setting('flexi.tenant_token', true);
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
            
            PERFORM set_config('flexi.current_tenant', tenant_id_val, true);
        END;
        $$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = flexi, pg_catalog, pg_temp;
    "#).await.unwrap();

    db.execute_unprepared(r#"
        CREATE OR REPLACE FUNCTION flexi.authorized_tenant_id() RETURNS text AS $$
        BEGIN
            RETURN current_setting('flexi.current_tenant', true);
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
            USING (tenant_id = flexi.authorized_tenant_id())
            WITH CHECK (tenant_id = flexi.authorized_tenant_id());
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
    use sea_orm::{Statement, DbBackend};
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
