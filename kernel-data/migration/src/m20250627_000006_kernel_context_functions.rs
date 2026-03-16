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
    -- Use gen_random_uuid() which generates UUIDv4 natively in Postgres 13+.
    -- For an audit log this provides sufficient uniqueness.
    v_id := pg_catalog.gen_random_uuid();

    -- Ensure the INSERT successfully passes RLS if the function owner lacks BYPASSRLS
    PERFORM set_config('flexi.current_tenant', 'system', true);

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
        now(),
        NULL
    );
END;
$$;

REVOKE ALL ON FUNCTION flexi.log_privileged_audit(text, text, jsonb) FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi_kernel_admin') THEN
        GRANT EXECUTE ON FUNCTION flexi.log_privileged_audit(text, text, jsonb) TO flexi_kernel_admin;
    END IF;
END $$;
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
