pub use sea_orm_migration::prelude::*;

mod m20240216_000001_init_rls;
mod m20240216_000002_create_entity_records;
mod m20240216_000003_create_audit_tables;
mod m20240520_000001_create_event_system;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240216_000001_init_rls::Migration),
            Box::new(m20240216_000002_create_entity_records::Migration),
            Box::new(m20240216_000003_create_audit_tables::Migration),
            Box::new(m20240520_000001_create_event_system::Migration),
        ]
    }
}
