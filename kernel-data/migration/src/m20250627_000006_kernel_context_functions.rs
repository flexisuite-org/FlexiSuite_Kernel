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
    -- NOTE: The Rust codebase uses UUIDv7 (Uuid::now_v7()) for time-ordered IDs.
    -- This function uses UUIDv4 because it runs inside a SECURITY DEFINER PL/pgSQL
    -- context where pg_uuidv7 extensions may not be available. The randomness is
    -- sufficient for audit log uniqueness, though a future migration to UUIDv7
    -- would improve index locality for chronological queries.
    v_id := pg_catalog.gen_random_uuid();

    -- Ensure the INSERT successfully passes RLS if the function owner lacks BYPASSRLS
    PERFORM set_config('flexi.current_tenant', 'system', true);
    IF current_setting('flexi.hmac_secret', true) IS NULL THEN
        RAISE EXCEPTION 'flexi.hmac_secret is not set; cannot compute ctx_sig. Configure the GUC for the connecting role before using this function.';
    END IF;
    PERFORM set_config('flexi.ctx_sig', encode(hmac('system', current_setting('flexi.hmac_secret', true), 'sha256'), 'hex'), true);

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

    -- Clear the GUCs so they do not leak into subsequent queries in the same transaction
    PERFORM set_config('flexi.current_tenant', '', true);
    PERFORM set_config('flexi.ctx_sig', '', true);
END;
$$;

REVOKE ALL ON FUNCTION flexi.log_privileged_audit(text, text, jsonb) FROM PUBLIC;

-- Create a dedicated NOLOGIN role to own SECURITY DEFINER functions.
-- This ensures the function's effective privileges are independent of
-- the migration runner role, preventing environment-dependent behavior.
DO $$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi_kernel_definer') THEN
        CREATE ROLE flexi_kernel_definer NOLOGIN NOINHERIT;
        RAISE NOTICE 'Created flexi_kernel_definer NOLOGIN role for SECURITY DEFINER function ownership.';
    END IF;
END $$;

-- Transfer ownership so the function runs as flexi_kernel_definer, not the migration role.
ALTER FUNCTION flexi.log_privileged_audit(text, text, jsonb) OWNER TO flexi_kernel_definer;

-- Grant USAGE on schema and INSERT on audit_logs so the NOLOGIN owner can operate.
GRANT USAGE ON SCHEMA flexi TO flexi_kernel_definer;
GRANT INSERT ON flexi.audit_logs TO flexi_kernel_definer;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi_kernel_admin') THEN
        GRANT EXECUTE ON FUNCTION flexi.log_privileged_audit(text, text, jsonb) TO flexi_kernel_admin;
    ELSE
        RAISE WARNING 'Role flexi_kernel_admin does not exist; flexi.log_privileged_audit has no grantees. Create the role before deploying this migration.';
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
