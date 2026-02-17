pub use sea_orm_migration::prelude::*;

mod m20240216_000001_init_rls;
mod m20240216_000002_create_entity_records;
mod m20250627_000003_create_diagnostics;
mod m20240216_000003_create_audit_tables;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240216_000001_init_rls::Migration),
            Box::new(m20240216_000002_create_entity_records::Migration),
            Box::new(m20250627_000003_create_diagnostics::Migration),
            Box::new(m20240216_000003_create_audit_tables::Migration),
        ]
    }
}
