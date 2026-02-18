use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = crate::MigrationConnection::new(manager.get_connection());

        // 1. Create diagnostic_policies table (flexi schema)
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.diagnostic_policies (
                tenant_id TEXT NOT NULL,
                enabled BOOLEAN NOT NULL DEFAULT FALSE,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_by TEXT,
                PRIMARY KEY (tenant_id)
            );
            "#,
        )
        .await?;

        // 2. Create diagnostic_reports table (flexi schema, composite PK)
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.diagnostic_reports (
                trace_id TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                error_code TEXT NOT NULL,
                context JSONB NOT NULL,
                suggestion TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (trace_id, tenant_id)
            );
            "#,
        )
        .await?;

        // 3. Enable and force RLS for diagnostic_reports
        db.execute_unprepared(
            r#"
            ALTER TABLE flexi.diagnostic_reports ENABLE ROW LEVEL SECURITY;
            ALTER TABLE flexi.diagnostic_reports FORCE ROW LEVEL SECURITY;
            DROP POLICY IF EXISTS tenant_isolation ON flexi.diagnostic_reports;
            DROP POLICY IF EXISTS tenant_isolation_policy ON flexi.diagnostic_reports;
            CREATE POLICY tenant_isolation ON flexi.diagnostic_reports
                FOR ALL
                TO PUBLIC
                USING (tenant_id = flexi.authorized_tenant_id());
            "#,
        )
        .await?;

        // 4. Enable and force RLS for diagnostic_policies
        db.execute_unprepared(
            r#"
            ALTER TABLE flexi.diagnostic_policies ENABLE ROW LEVEL SECURITY;
            ALTER TABLE flexi.diagnostic_policies FORCE ROW LEVEL SECURITY;
            DROP POLICY IF EXISTS tenant_isolation ON flexi.diagnostic_policies;
            DROP POLICY IF EXISTS tenant_isolation_policy ON flexi.diagnostic_policies;
            CREATE POLICY tenant_isolation ON flexi.diagnostic_policies
                FOR ALL
                TO PUBLIC
                USING (tenant_id = flexi.authorized_tenant_id());
            "#,
        )
        .await?;

        // 5. Create index with tenant_id as leading column for diagnostic_reports.
        // Note: diagnostic_policies already has PRIMARY KEY (tenant_id),
        // so a separate index would be redundant.
        db.execute_unprepared(
            r#"
            CREATE INDEX IF NOT EXISTS idx_diagnostic_reports_tenant_id
                ON flexi.diagnostic_reports (tenant_id, created_at);
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = crate::MigrationConnection::new(manager.get_connection());

        // Drop RLS policies before tables
        db.execute_unprepared(
            r#"
            DROP POLICY IF EXISTS tenant_isolation ON flexi.diagnostic_reports;
            DROP POLICY IF EXISTS tenant_isolation_policy ON flexi.diagnostic_reports;
            DROP POLICY IF EXISTS tenant_isolation ON flexi.diagnostic_policies;
            DROP POLICY IF EXISTS tenant_isolation_policy ON flexi.diagnostic_policies;
            "#,
        )
        .await?;

        db.execute_unprepared("DROP TABLE IF EXISTS flexi.diagnostic_reports")
            .await?;
        db.execute_unprepared("DROP TABLE IF EXISTS flexi.diagnostic_policies")
            .await?;

        Ok(())
    }
}
