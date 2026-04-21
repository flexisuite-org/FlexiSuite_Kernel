use sea_orm_migration::prelude::*;

/// ## Migration: Kernel Context Functions (Privileged Definer)
///
/// Creates the `flexi.log_privileged_audit` privileged-definer function for
/// privileged audit logging from the kernel background runner.
///
/// ### Deployment Requirements
/// - The migration runner must have `CREATEROLE` privilege (to create `flexi_kernel_definer`).
/// - The `flexi_kernel_admin` role must exist before running this migration.
/// - The `flexi.hmac_secret` GUC must be configured for the connecting role.
///
/// ### Ownership Model
/// Function ownership is transferred to `flexi_kernel_definer` (NOLOGIN, NOINHERIT)
/// so that effective privileges are environment-independent, not tied to the migration runner.
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
    -- This privileged audit log is written inside the DB by a SECURITY DEFINER
    -- function. gen_random_uuid() is intentional here: UUIDv4 provides sufficient
    -- uniqueness without requiring a pg_uuidv7 extension in production databases.
    v_id := pg_catalog.gen_random_uuid();

    -- Ensure the INSERT successfully passes RLS if the function owner lacks BYPASSRLS
    PERFORM set_config('flexi.current_tenant', 'system', true);
    IF current_setting('flexi.hmac_secret', true) IS NULL OR current_setting('flexi.hmac_secret', true) = '' THEN
        RAISE EXCEPTION 'flexi.hmac_secret is not set or empty; cannot compute ctx_sig. Configure the GUC for the connecting role before using this function.';
    END IF;
    -- pgcrypto is installed into the flexi schema by m20240216_000001_init_rls.rs,
    -- and this function's search_path includes flexi before pg_catalog.
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

    -- These SET LOCAL writes use set_config(..., true), so they are transaction-scoped
    -- and PostgreSQL auto-reverts them on rollback if the INSERT fails; this cleanup is
    -- defense-in-depth for the successful path, not required for correctness.
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

-- Grant schema privileges before ownership transfer; ALTER FUNCTION OWNER requires
-- the new owner to have CREATE privilege on the schema containing the function.
GRANT USAGE ON SCHEMA flexi TO flexi_kernel_definer;
GRANT CREATE ON SCHEMA flexi TO flexi_kernel_definer;

-- Transfer ownership so the function runs as flexi_kernel_definer, not the migration role.
ALTER FUNCTION flexi.log_privileged_audit(text, text, jsonb) OWNER TO flexi_kernel_definer;
REVOKE CREATE ON SCHEMA flexi FROM flexi_kernel_definer;

-- Grant INSERT on audit_logs so the NOLOGIN owner can operate.
GRANT INSERT ON flexi.audit_logs TO flexi_kernel_definer;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi_kernel_admin') THEN
        GRANT EXECUTE ON FUNCTION flexi.log_privileged_audit(text, text, jsonb) TO flexi_kernel_admin;
    ELSE
        RAISE WARNING 'Role flexi_kernel_admin does not exist; skipping GRANT for flexi.log_privileged_audit. Privileged audit logging will fail at runtime.';
    END IF;
END $$;
        "#;
        db.execute_unprepared(sql).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "DO $$
             BEGIN
               -- Only drop the role if it was created by this migration (owns the function).
               -- A deployment-provisioned role would not own this function.
               IF EXISTS (
                 SELECT 1
                 FROM pg_catalog.pg_proc
                 WHERE proname = 'log_privileged_audit'
                   AND pronamespace = 'flexi'::regnamespace
                   AND proowner = (SELECT oid FROM pg_catalog.pg_roles WHERE rolname = 'flexi_kernel_definer')
               ) THEN
                 DROP FUNCTION IF EXISTS flexi.log_privileged_audit(text, text, jsonb);
                 REVOKE ALL ON SCHEMA flexi FROM flexi_kernel_definer;
                 REVOKE ALL ON flexi.audit_logs FROM flexi_kernel_definer;
                 DROP ROLE flexi_kernel_definer;
               ELSE
                 DROP FUNCTION IF EXISTS flexi.log_privileged_audit(text, text, jsonb);
               END IF;
             END $$;",
        )
            .await?;
        Ok(())
    }
}
