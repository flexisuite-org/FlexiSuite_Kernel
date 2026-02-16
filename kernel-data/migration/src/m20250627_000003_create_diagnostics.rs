use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create diagnostic_policies table
        manager.create_table(
            Table::create()
                .table(DiagnosticPolicy::Table)
                .if_not_exists()
                .col(ColumnDef::new(DiagnosticPolicy::TenantId).string().not_null().primary_key())
                .col(ColumnDef::new(DiagnosticPolicy::Enabled).boolean().not_null().default(false))
                .col(ColumnDef::new(DiagnosticPolicy::UpdatedAt).timestamp_with_time_zone().not_null())
                .col(ColumnDef::new(DiagnosticPolicy::UpdatedBy).string()) // Nullable
                .to_owned(),
        ).await?;

        // Create diagnostic_reports table
        manager.create_table(
            Table::create()
                .table(DiagnosticReport::Table)
                .if_not_exists()
                .col(ColumnDef::new(DiagnosticReport::TraceId).string().not_null().primary_key())
                .col(ColumnDef::new(DiagnosticReport::TenantId).string().not_null())
                .col(ColumnDef::new(DiagnosticReport::ErrorCode).string().not_null())
                .col(ColumnDef::new(DiagnosticReport::Context).json().not_null())
                .col(ColumnDef::new(DiagnosticReport::Suggestion).string())
                .col(ColumnDef::new(DiagnosticReport::CreatedAt).timestamp_with_time_zone().not_null())
                .to_owned(),
        ).await?;

        // Note: RLS policies require superuser or specific permissions, typically handled in a separate step or assume DB user has perm.
        // Assuming we can run raw SQL.

        // Enable RLS for diagnostic_reports
        manager.get_connection().execute_unprepared("ALTER TABLE diagnostic_reports ENABLE ROW LEVEL SECURITY;").await?;
        manager.get_connection().execute_unprepared(
            "CREATE POLICY tenant_isolation ON diagnostic_reports
             USING (tenant_id = flexi.authorized_tenant_id());"
        ).await?;

        // Enable RLS for diagnostic_policies
        manager.get_connection().execute_unprepared("ALTER TABLE diagnostic_policies ENABLE ROW LEVEL SECURITY;").await?;
        manager.get_connection().execute_unprepared(
            "CREATE POLICY tenant_isolation ON diagnostic_policies
             USING (tenant_id = flexi.authorized_tenant_id());"
        ).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(DiagnosticReport::Table).to_owned()).await?;
        manager.drop_table(Table::drop().table(DiagnosticPolicy::Table).to_owned()).await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum DiagnosticPolicy {
    #[sea_orm(iden = "diagnostic_policies")]
    Table,
    TenantId,
    Enabled,
    UpdatedAt,
    UpdatedBy,
}

#[derive(DeriveIden)]
enum DiagnosticReport {
    #[sea_orm(iden = "diagnostic_reports")]
    Table,
    TraceId,
    TenantId,
    ErrorCode,
    Context,
    Suggestion,
    CreatedAt,
}
