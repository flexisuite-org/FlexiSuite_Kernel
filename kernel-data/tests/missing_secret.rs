use kernel_core::auth::{TenantContext, TenantId, UserId};
use kernel_data::connection::with_tenant_tx;
use sea_orm::Database;
use testcontainers::{RunnableImage, clients};
use testcontainers_modules::postgres::Postgres;

#[tokio::test(flavor = "multi_thread")]
async fn test_transaction_fails_without_secret_init() {
    // 1. Setup DB (Needed because with_tenant_tx starts a transaction before checking secret)
    let docker = clients::Cli::default();
    let image = RunnableImage::from(Postgres::default()).with_tag("15-alpine");
    let node = docker.run(image);
    let port = node.get_host_port_ipv4(5432);
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to connect to DB");

    // 2. Prepare Context
    // Note: We intentionally DO NOT call init_hmac_secret_for_test here.

    let tenant_id = TenantId::new("tenant-x").unwrap();
    let ctx = TenantContext::new(tenant_id, Some(UserId::new("user-1").unwrap()));

    // 3. Call with_tenant_tx
    let res = with_tenant_tx(&db, &ctx, |_| Box::pin(async { Ok(()) })).await;

    // 4. Verification
    assert!(res.is_err(), "Should fail when HMAC secret is missing");
    let err = res.unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("HMAC secret not initialized"),
        "Error message should indicate missing secret, got: {}",
        msg
    );
}
