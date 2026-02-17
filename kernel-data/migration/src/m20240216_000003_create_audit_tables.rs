use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // 1. Create Entity History Table
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.entity_histories (
                id TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                entity_id TEXT NOT NULL,
                entity_type TEXT NOT NULL,
                change_type TEXT NOT NULL,
                version INT NOT NULL,
                diff JSONB NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                created_by TEXT NOT NULL,
                archived_at TIMESTAMPTZ,
                PRIMARY KEY (id, tenant_id)
            );
            "#,
        )
        .await?;

        // Backfill and enforce non-null actor for existing rows.
        db.execute_unprepared(
            r#"
            UPDATE flexi.entity_histories
                SET created_by = 'system'
                WHERE created_by IS NULL;
            ALTER TABLE flexi.entity_histories
                ALTER COLUMN created_by SET NOT NULL;
            "#,
        )
        .await?;

        // Enforce DB-level referential integrity for entity history.
        db.execute_unprepared(
            r#"
            ALTER TABLE flexi.entity_histories
                DROP CONSTRAINT IF EXISTS fk_entity_histories_entity_record;
            ALTER TABLE flexi.entity_histories
                ADD CONSTRAINT fk_entity_histories_entity_record
                FOREIGN KEY (entity_id, tenant_id)
                REFERENCES flexi.entity_records (id, tenant_id)
                ON DELETE NO ACTION;
            "#,
        )
        .await?;

        // 2. Create Audit Log Table
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.audit_logs (
                id TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                action TEXT NOT NULL,
                resource TEXT NOT NULL,
                details JSONB NOT NULL,
                ip_address TEXT,
                user_agent TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                archived_at TIMESTAMPTZ,
                PRIMARY KEY (id, tenant_id)
            );
            "#,
        )
        .await?;

        // 3. Enable RLS for History
        db.execute_unprepared(
            r#"
            ALTER TABLE flexi.entity_histories ENABLE ROW LEVEL SECURITY;
            ALTER TABLE flexi.entity_histories FORCE ROW LEVEL SECURITY;
            DROP POLICY IF EXISTS tenant_isolation_policy ON flexi.entity_histories;
            CREATE POLICY tenant_isolation_policy ON flexi.entity_histories
                FOR ALL
                TO PUBLIC
                USING (tenant_id = flexi.authorized_tenant_id());
            "#,
        )
        .await?;

        // 4. Enable RLS for Audit Logs
        db.execute_unprepared(
            r#"
            ALTER TABLE flexi.audit_logs ENABLE ROW LEVEL SECURITY;
            ALTER TABLE flexi.audit_logs FORCE ROW LEVEL SECURITY;
            DROP POLICY IF EXISTS tenant_isolation_policy ON flexi.audit_logs;
            CREATE POLICY tenant_isolation_policy ON flexi.audit_logs
                FOR ALL
                TO PUBLIC
                USING (tenant_id = flexi.authorized_tenant_id());
            "#,
        )
        .await?;

        // 5. Create Indexes
        db.execute_unprepared(
            r#"
            CREATE INDEX IF NOT EXISTS idx_entity_histories_entity_id
                ON flexi.entity_histories (tenant_id, entity_id);
            CREATE INDEX IF NOT EXISTS idx_entity_histories_created_at
                ON flexi.entity_histories (tenant_id, created_at);
            CREATE INDEX IF NOT EXISTS idx_entity_histories_unarchived
                ON flexi.entity_histories (tenant_id, created_at)
                WHERE archived_at IS NULL;

            CREATE INDEX IF NOT EXISTS idx_audit_logs_actor_id
                ON flexi.audit_logs (tenant_id, actor_id);
            CREATE INDEX IF NOT EXISTS idx_audit_logs_created_at
                ON flexi.audit_logs (tenant_id, created_at);
            CREATE INDEX IF NOT EXISTS idx_audit_logs_unarchived
                ON flexi.audit_logs (tenant_id, created_at)
                WHERE archived_at IS NULL;
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TABLE IF EXISTS flexi.entity_histories")
            .await?;
        db.execute_unprepared("DROP TABLE IF EXISTS flexi.audit_logs")
            .await?;
        Ok(())
    }
}
