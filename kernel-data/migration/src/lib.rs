pub use sea_orm_migration::prelude::*;

mod m20240216_000001_init_rls;
mod m20240216_000002_create_entity_records;
mod m20240216_000003_create_audit_tables;
mod m20240520_000001_create_event_system;
mod m20250627_000004_create_diagnostics;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240216_000001_init_rls::Migration),
            Box::new(m20240216_000002_create_entity_records::Migration),
            Box::new(m20240216_000003_create_audit_tables::Migration),
Box::new(m20240520_000001_create_event_system::Migration),
            Box::new(m20250627_000004_create_diagnostics::Migration),
        ]
    }
}

pub struct MigrationConnection<'a, C>(pub &'a C)
where
    C: sea_orm_migration::sea_orm::ConnectionTrait;

impl<'a, C> MigrationConnection<'a, C>
where
    C: sea_orm_migration::sea_orm::ConnectionTrait,
{
    pub fn new(db: &'a C) -> Self {
        Self(db)
    }

    pub async fn execute_unprepared(
        &self,
        sql: &str,
    ) -> Result<sea_orm_migration::sea_orm::ExecResult, sea_orm_migration::sea_orm::DbErr> {
        self.0.execute_unprepared(sql).await
    }
}
