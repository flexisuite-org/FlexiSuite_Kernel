use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = crate::MigrationConnection::new(manager.get_connection());

        // 1. Create roles table with unique constraint on (tenant_id, name)
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.roles (
                id UUID NOT NULL DEFAULT gen_random_uuid(),
                tenant_id TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (id),
                UNIQUE (tenant_id, name)
            );
            "#,
        )
        .await?;

        // 2. Create groups table with unique constraint on (tenant_id, name)
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.groups (
                id UUID NOT NULL DEFAULT gen_random_uuid(),
                tenant_id TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (id),
                UNIQUE (tenant_id, name)
            );
            "#,
        )
        .await?;

        // 3. Create permissions table with unique constraint on (tenant_id, role_id, resource, action)
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.permissions (
                id UUID NOT NULL DEFAULT gen_random_uuid(),
                tenant_id TEXT NOT NULL,
                role_id UUID NOT NULL,
                resource TEXT NOT NULL,
                action TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (id),
                UNIQUE (tenant_id, role_id, resource, action)
            );
            "#,
        )
        .await?;

        // 4. Create role_members table (many-to-many between roles and groups/users)
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.role_members (
                id UUID NOT NULL DEFAULT gen_random_uuid(),
                tenant_id TEXT NOT NULL,
                role_id UUID NOT NULL,
                member_type TEXT NOT NULL, -- 'user' or 'group'
                member_id UUID NOT NULL,   -- references users.id or groups.id
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (id),
                UNIQUE (tenant_id, role_id, member_type, member_id)
            );
            "#,
        )
        .await?;

        // 5. Enable RLS for all tables
        for table in &["roles", "groups", "permissions", "role_members"] {
            db.execute_unprepared(&format!(
                "ALTER TABLE flexi.{} ENABLE ROW LEVEL SECURITY;",
                table
            ))
            .await?;
            db.execute_unprepared(&format!(
                "ALTER TABLE flexi.{} FORCE ROW LEVEL SECURITY;",
                table
            ))
            .await?;
        }

        // 6. Create RLS policies for roles
        db.execute_unprepared(
            r#"
            DROP POLICY IF EXISTS roles_tenant_isolation ON flexi.roles;
            CREATE POLICY roles_tenant_isolation ON flexi.roles
                FOR ALL
                TO PUBLIC
                USING (tenant_id = flexi.authorized_tenant_id());
            "#,
        )
        .await?;

        // 7. Create RLS policies for groups
        db.execute_unprepared(
            r#"
            DROP POLICY IF EXISTS groups_tenant_isolation ON flexi.groups;
            CREATE POLICY groups_tenant_isolation ON flexi.groups
                FOR ALL
                TO PUBLIC
                USING (tenant_id = flexi.authorized_tenant_id());
            "#,
        )
        .await?;

        // 8. Create RLS policies for permissions
        db.execute_unprepared(
            r#"
            DROP POLICY IF EXISTS permissions_tenant_isolation ON flexi.permissions;
            CREATE POLICY permissions_tenant_isolation ON flexi.permissions
                FOR ALL
                TO PUBLIC
                USING (tenant_id = flexi.authorized_tenant_id());
            "#,
        )
        .await?;

        // 9. Create RLS policies for role_members
        db.execute_unprepared(
            r#"
            DROP POLICY IF EXISTS role_members_tenant_isolation ON flexi.role_members;
            CREATE POLICY role_members_tenant_isolation ON flexi.role_members
                FOR ALL
                TO PUBLIC
                USING (tenant_id = flexi.authorized_tenant_id());
            "#,
        )
        .await?;

        // 10. Add foreign key constraints
        db.execute_unprepared(
            r#"
            ALTER TABLE flexi.permissions
                ADD CONSTRAINT fk_permissions_role_id
                FOREIGN KEY (role_id) REFERENCES flexi.roles(id) ON DELETE CASCADE;
            "#,
        )
        .await?;

        db.execute_unprepared(
            r#"
            ALTER TABLE flexi.role_members
                ADD CONSTRAINT fk_role_members_role_id
                FOREIGN KEY (role_id) REFERENCES flexi.roles(id) ON DELETE CASCADE;
            "#,
        )
        .await?;

        // 11. Create indexes for efficient querying
        db.execute_unprepared(
            r#"
            CREATE INDEX IF NOT EXISTS idx_roles_tenant_id ON flexi.roles (tenant_id);
            "#,
        )
        .await?;

        db.execute_unprepared(
            r#"
            CREATE INDEX IF NOT EXISTS idx_groups_tenant_id ON flexi.groups (tenant_id);
            "#,
        )
        .await?;

        db.execute_unprepared(
            r#"
            CREATE INDEX IF NOT EXISTS idx_permissions_tenant_id_role_id 
                ON flexi.permissions (tenant_id, role_id);
            "#,
        )
        .await?;

        db.execute_unprepared(
            r#"
            CREATE INDEX IF NOT EXISTS idx_role_members_role_id 
                ON flexi.role_members (role_id);
            "#,
        )
        .await?;
        db.execute_unprepared(
            r#"
            CREATE INDEX IF NOT EXISTS idx_role_members_tenant_id 
                ON flexi.role_members (tenant_id);
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = crate::MigrationConnection::new(manager.get_connection());

        // Drop indexes first
        db.execute_unprepared("DROP INDEX IF EXISTS flexi.idx_role_members_tenant_id;")
            .await?;
        db.execute_unprepared("DROP INDEX IF EXISTS flexi.idx_role_members_role_id;")
            .await?;
        db.execute_unprepared("DROP INDEX IF EXISTS flexi.idx_permissions_tenant_id_role_id;")
            .await?;
        db.execute_unprepared("DROP INDEX IF EXISTS flexi.idx_groups_tenant_id;")
            .await?;
        db.execute_unprepared("DROP INDEX IF EXISTS flexi.idx_roles_tenant_id;")
            .await?;

        // Drop foreign key constraints
        db.execute_unprepared(
            "ALTER TABLE flexi.role_members DROP CONSTRAINT IF EXISTS fk_role_members_role_id;",
        )
        .await?;
        db.execute_unprepared(
            "ALTER TABLE flexi.permissions DROP CONSTRAINT IF EXISTS fk_permissions_role_id;",
        )
        .await?;

        // Drop tables
        db.execute_unprepared("DROP TABLE IF EXISTS flexi.role_members;")
            .await?;
        db.execute_unprepared("DROP TABLE IF EXISTS flexi.permissions;")
            .await?;
        db.execute_unprepared("DROP TABLE IF EXISTS flexi.groups;")
            .await?;
        db.execute_unprepared("DROP TABLE IF EXISTS flexi.roles;")
            .await?;

        Ok(())
    }
}
