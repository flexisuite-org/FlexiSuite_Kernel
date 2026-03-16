use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let sql = r#"
CREATE OR REPLACE FUNCTION flexi.log_privileged_audit(
    p_action text,
    p_resource text,
    p_details jsonb
)
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = flexi, pg_catalog, pg_temp
AS $$
DECLARE
    v_id uuid;
BEGIN
    -- Use gen_random_uuid() for uuidv7-like or fallback to uuid-ossp if gen_random_uuid not generating v7
    -- PostgreSQL 13+ supports gen_random_uuid() which generates v4. Let's rely on pg_catalog.gen_random_uuid()
    v_id := pg_catalog.gen_random_uuid();

    INSERT INTO flexi.audit_logs (
        id, tenant_id, actor_id, action, resource, details, ip_address, user_agent, created_at, archived_at
    ) VALUES (
        v_id,
        'system',
        'kernel_admin',
        p_action,
        p_resource,
        p_details,
        NULL,
        'kernel-background-runner',
        pg_catalog.current_timestamp,
        NULL
    );
END;
$$;

REVOKE ALL ON FUNCTION flexi.log_privileged_audit(text, text, jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION flexi.log_privileged_audit(text, text, jsonb) TO flexi_kernel_admin;
        "#;
        db.execute_unprepared(sql).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP FUNCTION IF EXISTS flexi.log_privileged_audit(text, text, jsonb);")
            .await?;
        Ok(())
    }
}
