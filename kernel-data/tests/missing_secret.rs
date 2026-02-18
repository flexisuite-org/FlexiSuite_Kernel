use kernel_data::auth_context::{TenantContext, TenantId, UserId};
mod common;
use common::auth::TestAuth;
use kernel_data::connection::with_tenant_tx;
use migration::MigratorTrait;
use sea_orm::{ConnectionTrait, Database};
use testcontainers::{RunnableImage, clients};
use testcontainers_modules::postgres::Postgres;

#[tokio::test(flavor = "multi_thread")]
async fn test_auth_failures() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432); // Removed .unwrap()
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to connect to DB");

    // Create Role
    db.execute_unprepared("DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi') THEN CREATE ROLE flexi; END IF; END $$;").await.unwrap();

    // Run Migrations (creates key table and authorized_tenant function)
    migration::Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");

    // Set Internal Secret
    db.execute_unprepared("ALTER ROLE postgres SET flexi.hmac_secret = 'internal-secret'")
        .await
        .unwrap();

    // Reconnect
    drop(db);
    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to reconnect");

    // Init Keys
    TestAuth::init_keys(&db).await.expect("Failed to init keys");

    let tenant_id = TenantId::new("tenant-x").unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), Some(UserId::new("user-1").unwrap()));

    // 1. Test: Invalid Token Format
    let res = with_tenant_tx(&db, &ctx, "invalid-token", |_| Box::pin(async { Ok(()) })).await;
    assert!(res.is_err(), "Should fail with invalid token format");

    // 2. Test: Token signed with unknown Key ID
    let current_ts = chrono::Utc::now().timestamp();
    let token = format!("v2:unknown-kid:{current_ts}:nonce:tenant-x:sig");
    let res = with_tenant_tx(&db, &ctx, &token, |_| Box::pin(async { Ok(()) })).await;
    assert!(res.is_err(), "Should fail with unknown kid");

    // 3. Test: Valid format, Invalid Signature
    // Generate a valid-ish token
    let real_token = TestAuth::generate_tenant_token(&db, &tenant_id)
        .await
        .unwrap();
    // Tamper with signature (last part)
    let parts: Vec<&str> = real_token.split(':').collect();
    let tampered_token = format!(
        "{}:{}:{}:{}:{}:bad_sig",
        parts[0], parts[1], parts[2], parts[3], parts[4]
    );

    let res = with_tenant_tx(&db, &ctx, &tampered_token, |_| Box::pin(async { Ok(()) })).await;
    assert!(res.is_err(), "Should fail with invalid signature");

    // 4. Test: Missing Internal Secret
    // Unset secret
    db.execute_unprepared("ALTER ROLE postgres RESET flexi.hmac_secret")
        .await
        .unwrap();

    // Reconnect
    drop(db);
    let db = Database::connect(&connection_string)
        .await
        .expect("Reconnect");

    let token = TestAuth::generate_tenant_token(&db, &tenant_id)
        .await
        .unwrap();

    let res = with_tenant_tx(&db, &ctx, &token, |_| Box::pin(async { Ok(()) })).await;
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(
        err.contains("Internal HMAC secret not set"),
        "Should fail due to missing internal secret: {}",
        err
    );
}
