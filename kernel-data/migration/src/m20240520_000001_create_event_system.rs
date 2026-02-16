use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // entity_event_seq
        manager.create_table(
            Table::create()
                .table(EntityEventSeq::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(EntityEventSeq::EntityId)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(EntityEventSeq::LastSeq)
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .to_owned(),
        ).await?;

        // causality_event_seq
        manager.create_table(
            Table::create()
                .table(CausalityEventSeq::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(CausalityEventSeq::CausalityKey)
                        .string()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(CausalityEventSeq::LastSeq)
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .to_owned(),
        ).await?;

        // outbox
        manager.create_table(
            Table::create()
                .table(Outbox::Table)
                .if_not_exists()
                .col(
                    ColumnDef::new(Outbox::EventId)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Outbox::OrderMode)
                        .string()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Outbox::EntityId)
                        .uuid(),
                )
                .col(
                    ColumnDef::new(Outbox::EntitySeq)
                        .big_integer(),
                )
                .col(
                    ColumnDef::new(Outbox::CausalityKey)
                        .string(),
                )
                .col(
                    ColumnDef::new(Outbox::CausalitySeq)
                        .big_integer(),
                )
                .col(
                    ColumnDef::new(Outbox::Payload)
                        .json_binary()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Outbox::CreatedAt)
                        .timestamp_with_time_zone()
                        .not_null()
                        .default(Expr::current_timestamp()),
                )
                .col(
                    ColumnDef::new(Outbox::PublishedAt)
                        .timestamp_with_time_zone(),
                )
                .to_owned(),
        ).await?;

        let db = manager.get_connection();

        // Unique Index with WHERE clause (Partial Index)
        db.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_outbox_entity_order ON outbox (entity_id, entity_seq) WHERE order_mode = 'entity';"
        ).await?;

        db.execute_unprepared(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_outbox_causality_order ON outbox (causality_key, causality_seq) WHERE order_mode = 'causality';"
        ).await?;

        // Add CHECK constraints via raw SQL
        // We drop constraints first if they exist to be safe or just add them. Adding them if not exists is tricky in raw SQL in Postgres without DO block.
        // But since this is a migration, we assume it's running fresh or once.

        // Postgres check constraints
        db.execute_unprepared(
            "ALTER TABLE outbox ADD CONSTRAINT check_outbox_order_mode CHECK (order_mode IN ('entity', 'causality'));"
        ).await?;

        db.execute_unprepared(
            r#"
            ALTER TABLE outbox ADD CONSTRAINT check_outbox_entity_fields CHECK (
                (order_mode = 'entity' AND entity_id IS NOT NULL AND entity_seq IS NOT NULL AND causality_key IS NULL AND causality_seq IS NULL) OR
                (order_mode = 'causality' AND causality_key IS NOT NULL AND causality_seq IS NOT NULL AND entity_id IS NULL AND entity_seq IS NULL)
            );
            "#
        ).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Outbox::Table).to_owned()).await?;
        manager.drop_table(Table::drop().table(CausalityEventSeq::Table).to_owned()).await?;
        manager.drop_table(Table::drop().table(EntityEventSeq::Table).to_owned()).await?;
        Ok(())
    }
}

#[derive(Iden)]
enum EntityEventSeq {
    Table,
    EntityId,
    LastSeq,
}

#[derive(Iden)]
enum CausalityEventSeq {
    Table,
    CausalityKey,
    LastSeq,
}

#[derive(Iden)]
enum Outbox {
    Table,
    EventId,
    OrderMode,
    EntityId,
    EntitySeq,
    CausalityKey,
    CausalitySeq,
    Payload,
    CreatedAt,
    PublishedAt,
}
