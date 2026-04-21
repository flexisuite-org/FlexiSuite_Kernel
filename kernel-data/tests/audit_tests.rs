use kernel_data::auth_context::{TenantContext, TenantId, UserId};
mod common;
use common::auth::TestAuth;
use kernel_data::connection::{RawConnection, TenantScoped, with_tenant_tx};
use kernel_data::entities::{audit_log, entity_history, entity_record};
use kernel_data::repository::TenantRepository;
use migration::MigratorTrait;
use sea_orm::{
    ActiveValue, ColumnTrait, ConnectionTrait, Database, EntityTrait, QueryFilter, QueryOrder,
};
use testcontainers::{ImageExt, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

// NOTE: This module intentionally performs direct SeaORM read-back queries for test verification.
// TODO: Keep production DB access strictly within TenantScoped/TenantContext APIs; do not copy these patterns outside tests.
const TEST_INTERNAL_SECRET: &str = "test_internal_secret_for_audit";

use kernel_data::kernel_context::create_background_runner_context;
use std::sync::Arc;

fn expected_actor_id(tenant_id: &TenantId, user_id: &UserId) -> String {
    let scoped = format!("{}:{}", tenant_id.as_str(), user_id.as_str());
    let digest = ring::digest::digest(&ring::digest::SHA256, scoped.as_bytes());
    format!("uidh:{}", hex::encode(digest.as_ref()))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_audit_log_creation() {
    let node = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("start postgres");
    let port = node.get_host_port_ipv4(5432).await.expect("get port");
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
        TEST_INTERNAL_SECRET.replace("'", "''")
    ))
    .await
    .expect("Failed to set secret");

    drop(db);
    let db = Database::connect(&connection_string)
        .await
        .expect("Reconnect failed");

    // Init Keys
    TestAuth::init_keys(&db).await.expect("Failed to init keys");

    // Tenant Context
    let tenant_id = TenantId::new("audit-tenant").unwrap();
    let user_id = UserId::new("user-audit").unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), Some(user_id.clone()));

    // 1. Create Entity -> Should create history
    let entity_id = Uuid::now_v7().to_string();
    let entity_id_clone = entity_id.clone();
    let token_create = TestAuth::generate_tenant_token(&db, &tenant_id)
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

    // Verification-only direct query. Never use unscoped direct queries in production code.
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
    let token_update = TestAuth::generate_tenant_token(&db, &tenant_id)
        .await
        .expect("Failed to gen token for update");
    with_tenant_tx(&db, &ctx, &token_update, |repo| {
        Box::pin(async move {
            let active_model = entity_record::ActiveModel {
                id: ActiveValue::Set(entity_id_clone.clone()),
                content: ActiveValue::Set(serde_json::json!({"val": 2})),
                version: ActiveValue::Set(1),
                ..Default::default()
            };
            repo.update_entity(&entity_id_clone, active_model).await?;
            Ok(())
        })
    })
    .await
    .expect("Update entity failed");

    // Verification-only direct query. Never use unscoped direct queries in production code.
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
    let token_audit = TestAuth::generate_tenant_token(&db, &tenant_id)
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

    // Verification-only direct query. Never use unscoped direct queries in production code.
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
    let other_token = TestAuth::generate_tenant_token(&db, &other_tenant_id)
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

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Requires Docker
async fn test_kernel_context_log_privileged_audit_integration() {
    let node = Postgres::default().with_tag("15-alpine").start().await.expect("start postgres");
    let port = node.get_host_port_ipv4(5432).await.expect("get port");
    let connection_string = format!("postgres://postgres:postgres@127.0.0.1:{}/postgres", port);

    let db = Database::connect(&connection_string)
        .await
        .expect("Failed to connect to DB");

    // Roles
    db.execute_unprepared("DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi') THEN CREATE ROLE flexi; END IF; END $$;").await.expect("Failed to create role flexi");
    db.execute_unprepared("DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi_kernel_admin') THEN CREATE ROLE flexi_kernel_admin LOGIN PASSWORD 'admin_pass'; END IF; END $$;").await.expect("Failed to create role flexi_kernel_admin");
    db.execute_unprepared("DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi_test_unprivileged') THEN CREATE ROLE flexi_test_unprivileged LOGIN PASSWORD 'password'; END IF; END $$;").await.expect("Failed to create unprivileged role");

    // Migrations
    migration::Migrator::up(&db, None)
        .await
        .expect("Failed to run migrations");

    let kernel_db_string = format!("postgres://flexi_kernel_admin:admin_pass@127.0.0.1:{}/postgres", port);
    let kernel_db = Arc::new(Database::connect(&kernel_db_string).await.expect("Failed to connect admin db"));

    // Ensure the flexi_kernel_admin role has USAGE on schema flexi and can access audit_logs
    db.execute_unprepared("GRANT USAGE ON SCHEMA flexi TO flexi_kernel_admin;").await.expect("grant usage");
    db.execute_unprepared("GRANT SELECT ON flexi.audit_logs TO flexi_kernel_admin;").await.expect("grant select");

    // We can execute the SECURITY DEFINER function via KernelContext
    let kernel_ctx = create_background_runner_context(kernel_db.clone());

    kernel_ctx.with_tx(|txn| {
        Box::pin(async move {
            kernel_data::kernel_context::KernelContext::log_privileged_audit(
                txn,
                "test_action".to_string(),
                "test_resource".to_string(),
                serde_json::json!({"key": "value"}),
            ).await
        })
    }).await.expect("Failed to log privileged audit");

    // Verify it was inserted via a superuser connection
    let logs = audit_log::Entity::find()
        .filter(audit_log::Column::TenantId.eq("system"))
        .all(&db)
        .await
        .expect("Failed to query audit logs");

    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].action, "test_action");
    assert_eq!(logs[0].actor_id, "kernel_admin");
    assert_eq!(logs[0].resource, "test_resource");

    // Connect with unprivileged role to ensure it gets rejected by RLS if inserted directly
    let unpriv_connection_string = format!("postgres://flexi_test_unprivileged:password@127.0.0.1:{}/postgres", port);
    let unpriv_db = Database::connect(&unpriv_connection_string)
        .await
        .expect("Failed to connect unprivileged DB");

    let log = audit_log::ActiveModel {
        id: ActiveValue::Set(Uuid::now_v7().to_string()),
        tenant_id: ActiveValue::Set("system".to_string()),
        actor_id: ActiveValue::Set("kernel_admin".to_string()),
        action: ActiveValue::Set("unprivileged_insert".to_string()),
        resource: ActiveValue::Set("test_resource".to_string()),
        details: ActiveValue::Set(serde_json::json!({"key": "value"})),
        ip_address: ActiveValue::NotSet,
        user_agent: ActiveValue::Set(Some("kernel-background-runner".to_string())),
        created_at: ActiveValue::Set(chrono::Utc::now().into()),
        archived_at: ActiveValue::NotSet,
    };

    let result = audit_log::Entity::insert(log).exec(&unpriv_db).await;
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("permission denied") || err.to_string().contains("violates row-level security policy"),
        "Unprivileged direct insert should fail due to RLS or permissions, but got: {:?}",
        err
    );

    // Verify that unprivileged role cannot EXECUTE the SECURITY DEFINER function.
    // This tests the REVOKE ALL ... FROM PUBLIC + GRANT EXECUTE TO flexi_kernel_admin boundary.
    let func_result = unpriv_db
        .execute_unprepared("SELECT flexi.log_privileged_audit('unprivileged_call', 'test', '{}'::jsonb)")
        .await;
    assert!(
        func_result.is_err(),
        "Unprivileged role should NOT be able to execute flexi.log_privileged_audit, but the call succeeded. Got: {:?}",
        func_result
    );
    let func_err = func_result.unwrap_err().to_string();
    assert!(
        func_err.contains("permission denied") || func_err.contains("does not exist"),
        "Expected permission denied for function execution, but got: {}",
        func_err
    );
}
