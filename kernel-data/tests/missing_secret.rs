use kernel_core::auth::{KeyManager, TenantContext, TenantId, UserId};
use kernel_data::connection::with_tenant_tx;
use sea_orm::{Database, ConnectionTrait};
use testcontainers::{clients, RunnableImage};
use testcontainers_modules::postgres::Postgres;
use migration::MigratorTrait;

#[tokio::test(flavor = "multi_thread")]
async fn test_auth_failures() {
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432); // Removed .unwrap()
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string).await.expect("Failed to connect to DB");

    // Create Role
    db.execute_unprepared("DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi') THEN CREATE ROLE flexi; END IF; END $$;").await.unwrap();

    // Run Migrations (creates key table and authorized_tenant function)
    migration::Migrator::up(&db, None).await.expect("Failed to run migrations");

    // Set Internal Secret
    db.execute_unprepared("ALTER ROLE postgres SET flexi.hmac_secret = 'internal-secret'").await.unwrap();
    
    // Reconnect
    drop(db);
    let db = Database::connect(&connection_string).await.expect("Failed to reconnect");

    // Init Keys
    KeyManager::rotate_keys(&db).await.expect("Failed to init keys");

    let tenant_id = TenantId::new("tenant-x").unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), Some(UserId::new("user-1").unwrap()));

    // 1. Test: Invalid Token Format
    let res = with_tenant_tx(&db, &ctx, "invalid-token", |_| Box::pin(async { Ok(()) })).await;
    assert!(res.is_err(), "Should fail with invalid token format");

    // 2. Test: Token signed with unknown Key ID
    let token = "v2:unknown-kid:1234567890:nonce:tenant-x:sig";
    let res = with_tenant_tx(&db, &ctx, token, |_| Box::pin(async { Ok(()) })).await;
    assert!(res.is_err(), "Should fail with unknown kid");
    // assert!(res.unwrap_err().to_string().contains("Invalid token format"), "Or format error if parsing fails");

    // 3. Test: Valid format, Invalid Signature
    // Generate a valid-ish token
    let real_token = KeyManager::generate_tenant_token(&db, "tenant-x").await.unwrap();
    // Tamper with signature (last part)
    let parts: Vec<&str> = real_token.split(':').collect();
    let tampered_token = format!("{}:{}:{}:{}:{}:bad_sig", parts[0], parts[1], parts[2], parts[3], parts[4]);

    let res = with_tenant_tx(&db, &ctx, &tampered_token, |_| Box::pin(async { Ok(()) })).await;
    assert!(res.is_err(), "Should fail with invalid signature");

    // 4. Test: Missing Internal Secret
    // Unset secret
    db.execute_unprepared("ALTER ROLE postgres RESET flexi.hmac_secret").await.unwrap();

    // Reconnect
    drop(db);
    let db = Database::connect(&connection_string).await.expect("Reconnect");

    let token = KeyManager::generate_tenant_token(&db, "tenant-x").await.unwrap();

    let res = with_tenant_tx(&db, &ctx, &token, |_| Box::pin(async { Ok(()) })).await;
    assert!(res.is_err());
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Internal HMAC secret not set"), "Should fail due to missing internal secret: {}", err);
}
