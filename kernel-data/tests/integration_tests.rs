//! Integration tests for tenant isolation and authorization.
//!
//! # TenantContext Exception Notice
//!
//! The setup helpers in this module (admin operations handled via `TestAdminTenantContext`)
//! intentionally bypass `TenantContext`. This is an **explicit, test-only exception** to the
//! project-wide rule that all DB access must go through `TenantContext`. These operations are
//! administrative bootstrap steps that have no tenant scope by nature (they run
//! as the superuser before any tenant exists). Production code MUST NOT follow
//! this pattern.
use kernel_data::auth_context::{TenantContext, TenantId, UserId};
mod common;
use common::admin::TestAdminTenantContext;
use common::auth::TestAuth;
use kernel_data::connection::{AuthenticatedScoped, RawConnection, TenantScoped, with_tenant_tx};
use kernel_data::entities::entity_record;
use kernel_data::rbac::seed_rbac_membership_for_tests;
use kernel_data::repository::TenantRepository;
use kernel_data::{DataError, RBACRepository};
use sea_orm::{
    ActiveValue, ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement,
    TransactionTrait,
};
use std::sync::OnceLock;
use testcontainers::{RunnableImage, clients};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

const TEST_INTERNAL_SECRET: &str = "test_internal_secret_123";

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

    let admin = TestAdminTenantContext::new(&db);

    // Verify Role Exists
    admin
        .create_role()
        .await
        .expect("Failed to create role flexi");

    // 2. Run Migrations
    admin
        .run_migrations()
        .await
        .expect("Failed to run migrations");

    // 3. Configure Internal Database Secret (GUC)
    admin
        .set_secret(TEST_INTERNAL_SECRET)
        .await
        .expect("Failed to set flexi.hmac_secret for node");

    // 4. Reconnect to ensure all pool connections pick up the new GUC
    drop(db);
    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to reconnect to DB");
    (db, node)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_rbac_integration_real_postgres() {
    let (db, _node) = setup_test_db().await;
    TestAuth::init_keys(&db).await.expect("Failed to init keys");

    let tenant_id = TenantId::new("tenant-rbac").unwrap();
    let user_id = UserId::new("user-rbac").unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), Some(user_id.clone()));

    let token = TestAuth::generate_tenant_token(&db, &tenant_id)
        .await
        .expect("gen token");

    with_tenant_tx(&db, &ctx, &token, |scoped| {
        let tenant_id = tenant_id.clone();
        let user_id = user_id.clone();
        Box::pin(async move {
            seed_rbac_membership_for_tests(scoped, &tenant_id, &user_id).await?;

            // Verify
            let auth_scoped = AuthenticatedScoped::try_from_scoped(scoped)
                .expect("user_id must be present in RBAC test context");
            let perms = RBACRepository::get_user_permissions(&auth_scoped).await?;
            assert_eq!(perms.len(), 1);
            assert_eq!(perms[0].resource, "res-1");
            assert_eq!(perms[0].action, "act-1");

            Ok(())
        })
    })
    .await
    .expect("RBAC test failed");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_tenant_isolation_rls() {
    let (db, _node) = setup_test_db().await;

    // 5. Initialize Keys (HMAC)
    TestAuth::init_keys(&db)
        .await
        .expect("Failed to initialize keys");

    // 6. Test Logic
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
    let token_a_1 = TestAuth::generate_tenant_token(&db, tenant_a.tenant_id())
        .await
        .expect("gen token A1");
    with_tenant_tx(&db, &tenant_a, &token_a_1, |repo| {
        Box::pin(async move { insert_record(repo, record_id_a, "bar").await })
    })
    .await
    .expect("Tenant A insert failed");

    // B. Read as Tenant A (Should succeed)
    let record_id_clone = record_id.clone();
    let token_a_2 = TestAuth::generate_tenant_token(&db, tenant_a.tenant_id())
        .await
        .expect("gen token A2");
    with_tenant_tx(&db, &tenant_a, &token_a_2, |repo| {
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
    let token_b_1 = TestAuth::generate_tenant_token(&db, tenant_b.tenant_id())
        .await
        .expect("gen token B1");
    with_tenant_tx(&db, &tenant_b, &token_b_1, |repo| {
        Box::pin(async move {
            let res = repo.get_entity(&record_id_clone).await?;
            assert!(res.is_none(), "Tenant B should NOT see Tenant A's record");
            Ok(())
        })
    })
    .await
    .expect("Tenant B read failed");

    // D. Insert as Tenant B (Should succeed independent of A)
    // Note: ID reuse across tenants?
    // entity_record PK is (id, tenant_id). So same ID is allowed for different tenant.
    let record_id_b = record_id.clone();
    let token_b_2 = TestAuth::generate_tenant_token(&db, tenant_b.tenant_id())
        .await
        .expect("gen token B2");
    with_tenant_tx(&db, &tenant_b, &token_b_2, |repo| {
        Box::pin(async move { insert_record(repo, record_id_b, "baz").await })
    })
    .await
    .expect("Tenant B insert failed");
}

async fn insert_record(
    repo: &TenantScoped<RawConnection>,
    id: String,
    val: &str,
) -> Result<(), DataError> {
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
#[ignore] // Requires Docker
async fn test_delete_entity_contract() {
    let (db, _node) = setup_test_db().await;
    TestAuth::init_keys(&db).await.expect("Failed to init keys");

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
    let token_a = TestAuth::generate_tenant_token(&db, tenant_a.tenant_id())
        .await
        .expect("gen token A");
    with_tenant_tx(&db, &tenant_a, &token_a, |repo| {
        Box::pin(async move { insert_record(repo, record_id_for_insert, "before-delete").await })
    })
    .await
    .expect("Tenant A insert failed");

    let record_id_for_cross_delete = record_id.clone();
    let token_b = TestAuth::generate_tenant_token(&db, tenant_b.tenant_id())
        .await
        .expect("gen token B");
    let cross_delete_result = with_tenant_tx(&db, &tenant_b, &token_b, |repo| {
        Box::pin(async move { repo.delete_entity(&record_id_for_cross_delete).await })
    })
    .await;
    assert!(
        matches!(
            cross_delete_result,
            Err(DataError::ValidationError(ref msg)) if msg == "Entity not found"
        ),
        "Cross-tenant delete must be treated as not found"
    );

    let record_id_for_delete = record_id.clone();
    let token_a_2 = TestAuth::generate_tenant_token(&db, tenant_a.tenant_id())
        .await
        .expect("gen token A2");
    with_tenant_tx(&db, &tenant_a, &token_a_2, |repo| {
        Box::pin(async move { repo.delete_entity(&record_id_for_delete).await })
    })
    .await
    .expect("Tenant A delete failed");

    let record_id_for_get = record_id.clone();
    let token_a_3 = TestAuth::generate_tenant_token(&db, tenant_a.tenant_id())
        .await
        .expect("gen token A3");
    with_tenant_tx(&db, &tenant_a, &token_a_3, |repo| {
        Box::pin(async move {
            let entity = repo.get_entity(&record_id_for_get).await?;
            assert!(entity.is_none(), "Deleted entity should not exist");
            Ok(())
        })
    })
    .await
    .expect("Tenant A get after delete failed");

    let missing_id = Uuid::now_v7().to_string();
    let token_a_4 = TestAuth::generate_tenant_token(&db, tenant_a.tenant_id())
        .await
        .expect("gen token A4");
    let missing_delete_result = with_tenant_tx(&db, &tenant_a, &token_a_4, |repo| {
        Box::pin(async move { repo.delete_entity(&missing_id).await })
    })
    .await;
    assert!(
        matches!(
            missing_delete_result,
            Err(DataError::ValidationError(ref msg)) if msg == "Entity not found"
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

    let admin = TestAdminTenantContext::new(&db);

    // Ensure role flexi does NOT exist
    let rows = admin
        .query_all_check("SELECT 1 FROM pg_roles WHERE rolname = 'flexi'")
        .await
        .expect("Failed to query roles");
    assert_eq!(rows.len(), 0, "Role flexi should not exist yet");

    // Run Migrations
    admin
        .run_migrations()
        .await
        .expect("Migration failed when role flexi is missing");

    // Verify schema exists
    let schema_exists = admin
        .query_all_check("SELECT 1 FROM information_schema.schemata WHERE schema_name = 'flexi'")
        .await
        .expect("Failed to query schema");
    assert_eq!(schema_exists.len(), 1, "Schema flexi should exist");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_authorize_rejects_nonce_reuse() {
    let (db, _node) = setup_test_db().await;
    TestAuth::init_keys(&db).await.expect("Failed to init keys");

    let tenant_id = TenantId::new("nonce-tenant").unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), Some(UserId::new("user-1").unwrap()));
    let token = TestAuth::generate_tenant_token(&db, &tenant_id)
        .await
        .expect("Failed to generate token");

    with_tenant_tx(&db, &ctx, &token, |_| Box::pin(async { Ok(()) }))
        .await
        .expect("First token usage should succeed");

    let second = with_tenant_tx(&db, &ctx, &token, |_| Box::pin(async { Ok(()) })).await;
    assert!(
        second.is_err(),
        "Second token usage must fail due to nonce reuse"
    );
    let err = second.unwrap_err().to_string();
    assert!(
        err.contains("Nonce already used"),
        "Expected 'Nonce already used' error, got: {}",
        err
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_authorized_tenant_id_rejects_manual_context_tampering() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to connect to DB");

    let admin = TestAdminTenantContext::new(&db);
    admin
        .create_role()
        .await
        .expect("Failed to create role flexi");
    admin
        .run_migrations()
        .await
        .expect("Failed to run migrations");

    // We use a transaction here to simulate a session where we try to tamper
    let txn = db.begin().await.expect("Failed to start transaction");

    // 1. Manually set the secret (simulating server config)
    txn.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT set_config('flexi.hmac_secret', $1, true)",
        [TEST_INTERNAL_SECRET.into()],
    ))
    .await
    .expect("Failed to set test secret in transaction");

    // 2. Manually set the tenant ID (simulating an attack trying to spoof a tenant)
    txn.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT set_config('flexi.current_tenant', 'attacker', true)",
        [],
    ))
    .await
    .expect("Failed to tamper tenant context");

    // 3. Try to use authorized_tenant_id() which should verify the signature
    let res = txn
        .query_one(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT flexi.authorized_tenant_id()",
            [],
        ))
        .await;

    assert!(res.is_err(), "Tampered context must be rejected");
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("Tenant context integrity check failed"),
        "Expected integrity error, got: {}",
        err
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_connection_rejects_colon_injection() {
    let (db, _node) = setup_test_db().await;
    TestAuth::init_keys(&db).await.expect("Failed to init keys");

    // Safety: Use test-only method to bypass validation instead of unsafe transmute
    let invalid = TenantId::new_unchecked("invalid:id");

    // We also need a TenantContext
    let ctx = TenantContext::new(invalid, None);

    let token = TestAuth::generate_tenant_token(&db, ctx.tenant_id())
        .await
        .expect("Failed to generate token");

    // Now call with_tenant_tx
    let res = with_tenant_tx(&db, &ctx, &token, |_| Box::pin(async { Ok(()) })).await;

    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("tenant_id must not contain ':'"),
        "Should match our new error check: {}",
        err
    );
}
