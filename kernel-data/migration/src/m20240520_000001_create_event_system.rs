use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.entity_event_seq (
                tenant_id TEXT NOT NULL,
                entity_id UUID NOT NULL,
                last_seq BIGINT NOT NULL DEFAULT 0,
                PRIMARY KEY (tenant_id, entity_id)
            );
            "#,
        )
        .await?;

        db.execute_unprepared(
            r#"
            ALTER TABLE flexi.entity_event_seq ENABLE ROW LEVEL SECURITY;
            ALTER TABLE flexi.entity_event_seq FORCE ROW LEVEL SECURITY;
            DROP POLICY IF EXISTS tenant_isolation_policy ON flexi.entity_event_seq;
            CREATE POLICY tenant_isolation_policy ON flexi.entity_event_seq
                FOR ALL
                TO PUBLIC
                USING (tenant_id = flexi.authorized_tenant_id());
            "#,
        )
        .await?;

        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.causality_event_seq (
                tenant_id TEXT NOT NULL,
                causality_key TEXT NOT NULL,
                last_seq BIGINT NOT NULL DEFAULT 0,
                PRIMARY KEY (tenant_id, causality_key)
            );
            "#,
        )
        .await?;

        db.execute_unprepared(
            r#"
            ALTER TABLE flexi.causality_event_seq ENABLE ROW LEVEL SECURITY;
            ALTER TABLE flexi.causality_event_seq FORCE ROW LEVEL SECURITY;
            DROP POLICY IF EXISTS tenant_isolation_policy ON flexi.causality_event_seq;
            CREATE POLICY tenant_isolation_policy ON flexi.causality_event_seq
                FOR ALL
                TO PUBLIC
                USING (tenant_id = flexi.authorized_tenant_id());
            "#,
        )
        .await?;

        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.outbox (
                event_id UUID NOT NULL,
                tenant_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                order_mode TEXT NOT NULL,
                entity_id UUID,
                entity_seq BIGINT,
                causality_key TEXT,
                causality_seq BIGINT,
                payload JSONB NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                published_at TIMESTAMPTZ,
                PRIMARY KEY (tenant_id, event_id)
            );
            "#,
        )
        .await?;

        db.execute_unprepared(
            r#"
            ALTER TABLE flexi.outbox ENABLE ROW LEVEL SECURITY;
            ALTER TABLE flexi.outbox FORCE ROW LEVEL SECURITY;
            DROP POLICY IF EXISTS tenant_isolation_policy ON flexi.outbox;
            CREATE POLICY tenant_isolation_policy ON flexi.outbox
                FOR ALL
                TO PUBLIC
                USING (tenant_id = flexi.authorized_tenant_id());
            "#,
        )
        .await?;

        db.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_outbox_entity_order ON flexi.outbox (tenant_id, entity_id, entity_seq) WHERE order_mode = 'entity';"
        )
        .await?;

        db.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_outbox_causality_order ON flexi.outbox (tenant_id, causality_key, causality_seq) WHERE order_mode = 'causality';"
        )
        .await?;

        db.execute_unprepared(
            r#"
            DO $$
            BEGIN
                IF NOT EXISTS (
                    SELECT 1 FROM pg_constraint
                    WHERE conname = 'check_outbox_order_mode'
                    AND conrelid = 'flexi.outbox'::regclass
                ) THEN
                    ALTER TABLE flexi.outbox
                    ADD CONSTRAINT check_outbox_order_mode
                    CHECK (order_mode IN ('entity', 'causality'));
                END IF;
            END $$;
            "#,
        )
        .await?;

        db.execute_unprepared(
            r#"
            DO $$
            BEGIN
                IF NOT EXISTS (
                    SELECT 1 FROM pg_constraint
                    WHERE conname = 'check_outbox_entity_fields'
                    AND conrelid = 'flexi.outbox'::regclass
                ) THEN
                    ALTER TABLE flexi.outbox
                    ADD CONSTRAINT check_outbox_entity_fields CHECK (
                        (order_mode = 'entity' AND entity_id IS NOT NULL AND entity_seq IS NOT NULL AND causality_key IS NULL AND causality_seq IS NULL) OR
                        (order_mode = 'causality' AND causality_key IS NOT NULL AND causality_seq IS NOT NULL AND entity_id IS NULL AND entity_seq IS NULL)
                    );
                END IF;
            END $$;
            "#
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TABLE IF EXISTS flexi.outbox CASCADE")
            .await?;
        db.execute_unprepared("DROP TABLE IF EXISTS flexi.causality_event_seq CASCADE")
            .await?;
        db.execute_unprepared("DROP TABLE IF EXISTS flexi.entity_event_seq CASCADE")
            .await?;
        Ok(())
    }
}
