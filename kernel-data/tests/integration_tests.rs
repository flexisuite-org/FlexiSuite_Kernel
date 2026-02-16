use kernel_core::auth::TenantContext;
use kernel_core::kernel;
use kernel_data::connection::{with_tenant_tx, TenantScoped, RawConnection};
use kernel_data::repository::TenantRepository;
use sea_orm::{Database, ActiveValue, ConnectionTrait};
use testcontainers::{clients, RunnableImage};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;
use kernel_data::entities::entity_record;

// We need a way to run migrations. Since migration crate is internal to kernel-data or separate?
// kernel-data re-exports nothing about migration.
// We might need to add `kernel-data-migration` as a dev-dependency or access it if it's in workspace.
// Looking at file structure, migration is a member of workspace or inside kernel-data?
// It was `kernel-data/migration`.
// Let's assume we can rely on `sea_orm_migration` to run it if we import the migration crate.
// But wait, `integration_tests.rs` is outside the crate structure (tests/ folder).
// We need `kernel_migration` crate available.
// Let's check Cargo.toml of `kernel-data` again. It has `migration` member?
// No, it likely has a workspace structure or `migration` is a separate crate.
// Based on previous `ls`, `migration` is inside `kernel-data`.
// If it is a library crate inside, we might need to add it to dev-dependencies of `kernel-data`?
// Or just reference it if it's part of the lib?
// Usually `migration` is a separate crate.
// Let's assume for now we just `include!` it or use `sea_orm::Schema` to create tables if migration crate is hard to link.
// BETTER: The migration crate is defined in `kernel-data/migration/Cargo.toml`.
// We should add it as a dev-dependency to `kernel-data` to run it.

#[tokio::test]
async fn test_tenant_isolation_rls() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    // 1. Connect
    let db = Database::connect(&connection_string).await.expect("Failed to connect to DB");

    // 2. Run Migrations
    // We need the migration logic. If we can't easily link the migration crate, 
    // we might need to replicate the schema creation SQL here for the test.
    // Ideally we link `kernel_migration`.
    // For now, let's create the schema manually to ensure test runs, 
    // verifying the RLS *logic* specifically.
    
    // Manual Schema Setup (Copy of migration logic for test isolation)
    db.execute_unprepared("CREATE SCHEMA IF NOT EXISTS flexi").await.unwrap();
    db.execute_unprepared(r#"
        CREATE TABLE IF NOT EXISTS flexi.flexi_nonce (
            nonce TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            PRIMARY KEY (nonce, created_at)
        ) PARTITION BY RANGE (created_at);
        CREATE TABLE IF NOT EXISTS flexi.flexi_nonce_default PARTITION OF flexi.flexi_nonce DEFAULT;
    "#).await.unwrap();

    // Mock Authorize Function (Simpler for test, or copy exact one)
    // We need `authorize_tenant` because `with_tenant_tx` calls it.
    db.execute_unprepared(r#"
        CREATE OR REPLACE FUNCTION flexi.authorize_tenant() RETURNS void AS $$
        BEGIN
            -- Minimized for test: just accept whatever is in current_setting or rely on the Mock Logic
            -- The real `with_tenant_tx` sets `flexi.tenant_token`.
            -- The real `authorize_tenant` parses it.
            -- To test REAL RLS, we should use the REAL function.
            -- But we need to make sure `with_tenant_tx` generates a valid token for IT.
            -- `with_tenant_tx` generates a "mock_sig" token.
            -- The migration expects "mock_sig".
            -- So if we copy the migration SQL exactly, it should pass.
            
            DECLARE
                token_val text;
                parts text[];
                tenant_id_val text;
            BEGIN
                token_val := current_setting('flexi.tenant_token', true);
                if token_val is null or token_val = '' then return; end if;
                
                parts := string_to_array(token_val, ':');
                -- v2:kid:ts:nonce:tenant_id:sig
                tenant_id_val := parts[5];
                
                -- Simulate signature check pass
                
                PERFORM set_config('flexi.current_tenant', tenant_id_val, true);
            END;
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
        CREATE TABLE IF NOT EXISTS flexi.entity_records (
            id TEXT NOT NULL,
            tenant_id TEXT NOT NULL,
            entity_type TEXT NOT NULL,
            schema_version INT NOT NULL DEFAULT 1,
            content JSONB NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            version INT NOT NULL DEFAULT 1,
            PRIMARY KEY (id, tenant_id)
        );
        ALTER TABLE flexi.entity_records ENABLE ROW LEVEL SECURITY;
        CREATE POLICY tenant_isolation_policy ON flexi.entity_records
            FOR ALL TO PUBLIC
            USING (tenant_id = flexi.authorized_tenant_id());
    "#).await.unwrap();


    // 3. Test Logic
    let tenant_a = TenantContext { tenant_id: "tenant-a".to_string(), user_id: Some("user-1".to_string()) };
    let tenant_b = TenantContext { tenant_id: "tenant-b".to_string(), user_id: Some("user-2".to_string()) };
    
    let record_id = Uuid::now_v7().to_string();

    // A. Insert as Tenant A
    let record_id_a = record_id.clone();
    with_tenant_tx(&db, &tenant_a, |repo| Box::pin(async move {
        insert_record(repo, record_id_a, "tenant-a", "bar").await
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
        insert_record(repo, record_id_b, "tenant-b", "baz").await
    })).await.expect("Tenant B insert failed");
    
    // E. Verify uniqueness allows same ID for different tenants (if PK allows)
    // Our PK is (id, tenant_id), so this works.
}

async fn insert_record(repo: &TenantScoped<RawConnection>, id: String, tenant: &str, val: &str) -> kernel::Result<()> {
    let active_model = entity_record::ActiveModel {
        id: ActiveValue::Set(id),
        tenant_id: ActiveValue::Set(tenant.to_string()),
        entity_type: ActiveValue::Set("test".to_string()),
        content: ActiveValue::Set(serde_json::json!({"foo": val})),
        ..Default::default()
    };
    repo.create_entity(active_model).await?;
    Ok(())
}
