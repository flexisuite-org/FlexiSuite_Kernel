use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = crate::MigrationConnection::new(manager.get_connection());

        // 1. Roles table
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.roles (
                id UUID NOT NULL,
                tenant_id TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (id, tenant_id)
            );
            "#,
        ).await?;

        // 2. Permissions table
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.permissions (
                id UUID NOT NULL,
                tenant_id TEXT NOT NULL,
                role_id UUID NOT NULL,
                resource TEXT NOT NULL,
                action TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (id, tenant_id),
                FOREIGN KEY (role_id, tenant_id) REFERENCES flexi.roles(id, tenant_id) ON DELETE CASCADE
            );
            "#,
        ).await?;

        // 3. Groups table
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.groups (
                id UUID NOT NULL,
                tenant_id TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (id, tenant_id)
            );
            "#,
        ).await?;

        // 4. Group Members table (User <-> Group)
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.group_members (
                id UUID NOT NULL,
                tenant_id TEXT NOT NULL,
                group_id UUID NOT NULL,
                user_id TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (id, tenant_id),
                FOREIGN KEY (group_id, tenant_id) REFERENCES flexi.groups(id, tenant_id) ON DELETE CASCADE,
                UNIQUE (tenant_id, group_id, user_id)
            );
            "#,
        ).await?;

        // 5. Group Roles table (Group <-> Role)
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.group_roles (
                id UUID NOT NULL,
                tenant_id TEXT NOT NULL,
                group_id UUID NOT NULL,
                role_id UUID NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (id, tenant_id),
                FOREIGN KEY (group_id, tenant_id) REFERENCES flexi.groups(id, tenant_id) ON DELETE CASCADE,
                FOREIGN KEY (role_id, tenant_id) REFERENCES flexi.roles(id, tenant_id) ON DELETE CASCADE,
                UNIQUE (tenant_id, group_id, role_id)
            );
            "#,
        ).await?;

        // Enable RLS for all tables
        let tables = ["roles", "permissions", "groups", "group_members", "group_roles"];
        for table in tables {
            db.execute_unprepared(&format!(
                r#"
                ALTER TABLE flexi.{table} ENABLE ROW LEVEL SECURITY;
                ALTER TABLE flexi.{table} FORCE ROW LEVEL SECURITY;
                DROP POLICY IF EXISTS tenant_isolation ON flexi.{table};
                CREATE POLICY tenant_isolation ON flexi.{table}
                    FOR ALL
                    TO PUBLIC
                    USING (tenant_id = flexi.authorized_tenant_id());
                "#
            )).await?;
        }

        // Add Indexes
        // Roles: tenant_id, name (for lookup)
        db.execute_unprepared("CREATE INDEX IF NOT EXISTS idx_roles_tenant_name ON flexi.roles (tenant_id, name)").await?;
        // Permissions: tenant_id, role_id (for fetch)
        db.execute_unprepared("CREATE INDEX IF NOT EXISTS idx_permissions_tenant_role ON flexi.permissions (tenant_id, role_id)").await?;
        // Group Members: tenant_id, user_id (to find user's groups)
        db.execute_unprepared("CREATE INDEX IF NOT EXISTS idx_group_members_tenant_user ON flexi.group_members (tenant_id, user_id)").await?;
        // Group Roles: tenant_id, group_id (to find group's roles)
        db.execute_unprepared("CREATE INDEX IF NOT EXISTS idx_group_roles_tenant_group ON flexi.group_roles (tenant_id, group_id)").await?;
        // Group Roles: tenant_id, role_id (to find groups with a specific role)
        db.execute_unprepared("CREATE INDEX IF NOT EXISTS idx_group_roles_tenant_role ON flexi.group_roles (tenant_id, role_id)").await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = crate::MigrationConnection::new(manager.get_connection());

        let tables = ["group_roles", "group_members", "groups", "permissions", "roles"];
        for table in tables {
            db.execute_unprepared(&format!("DROP TABLE IF EXISTS flexi.{table}")).await?;
        }

        Ok(())
    }
}
