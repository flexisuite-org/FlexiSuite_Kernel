use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // 1. Create Table
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.entity_records (
                id TEXT NOT NULL,
                tenant_id TEXT NOT NULL,
                entity_type TEXT NOT NULL,
                schema_version INT NOT NULL DEFAULT 1,
                content JSONB NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                version INT NOT NULL DEFAULT 1,
                PRIMARY KEY (id, tenant_id)
            );
            "#,
        )
        .await?;

        // 2. Enable RLS
        db.execute_unprepared(
            r#"
            ALTER TABLE flexi.entity_records ENABLE ROW LEVEL SECURITY;
            ALTER TABLE flexi.entity_records FORCE ROW LEVEL SECURITY;
            "#,
        )
        .await?;

        // 3. Create RLS Policy (drop-if-exists for idempotency)
        db.execute_unprepared(
            r#"
            DROP POLICY IF EXISTS tenant_isolation_policy ON flexi.entity_records;
            CREATE POLICY tenant_isolation_policy ON flexi.entity_records
                FOR ALL
                TO PUBLIC
                USING (tenant_id = flexi.authorized_tenant_id());
            "#,
        )
        .await?;

        // 4. Create Index
        db.execute_unprepared(
            r#"
            CREATE INDEX IF NOT EXISTS idx_entity_records_type
                ON flexi.entity_records (tenant_id, entity_type);
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TABLE IF EXISTS flexi.entity_records")
            .await?;
        Ok(())
    }
}
