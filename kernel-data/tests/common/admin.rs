use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};

/// A test-only helper that wraps `DatabaseConnection` to perform administrative
/// tasks that bypass tenant isolation (e.g., creating roles, running migrations).
///
/// This exists to enforce the rule that all DB access must go through a usage-specific
/// context (like `TenantContext`), while acknowledging that tests need to perform
/// setup steps that are inherently "super-user" and not tenant-scoped.
pub struct TestAdminTenantContext<'a> {
    db: &'a DatabaseConnection,
}

impl<'a> TestAdminTenantContext<'a> {
    pub fn new(db: &'a DatabaseConnection) -> Self {
        Self { db }
    }

    /// Creates the 'flexi' role if it does not exist.
    pub async fn create_role(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.db
            .execute_unprepared("DO $$ BEGIN IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi') THEN CREATE ROLE flexi; END IF; END $$;")
            .await?;
        Ok(())
    }

    /// Runs database migrations.
    pub async fn run_migrations(&self) -> Result<(), Box<dyn std::error::Error>> {
        use migration::MigratorTrait;
        migration::Migrator::up(self.db, None).await?;
        Ok(())
    }

    /// Sets a secret for the 'postgres' role.
    pub async fn set_secret(&self, secret: &str) -> Result<(), Box<dyn std::error::Error>> {
        // PostgreSQL's ALTER ROLE ... SET does not support parameters ($1).
        // Escape single quotes to keep SQL string literal boundaries intact.
        // WARNING: This escaping is acceptable only for test-only, compile-time-controlled
        // inputs. Callers MUST NEVER pass user-provided data into this method.
        let escaped_secret = secret.replace('\'', "''");

        self.db
            .execute_unprepared(&format!(
                "ALTER ROLE postgres SET flexi.hmac_secret = '{}'",
                escaped_secret
            ))
            .await?;
        Ok(())
    }

    /// Executes an unprepared SQL statement.
    ///
    /// # Safety
    /// This method allows executing arbitrary SQL, bypassing type checks and
    /// tenant isolation. It should ONLY be used for test setup/teardown or
    /// verifying specific security controls (e.g. attempting to tamper with
    /// session variables).
    pub async fn execute_unprepared_bloody_murder(
        &self,
        sql: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.db.execute_unprepared(sql).await?;
        Ok(())
    }

    /// Queries for scalar values using raw SQL.
    ///
    /// # Safety
    /// This method allows executing arbitrary SQL, bypassing type checks and
    /// tenant isolation. It should ONLY be used for test assertions.
    pub async fn query_one_check(
        &self,
        sql: &str,
    ) -> Result<Option<sea_orm::QueryResult>, Box<dyn std::error::Error>> {
        let res = self
            .db
            .query_one(Statement::from_string(DbBackend::Postgres, sql.to_owned()))
            .await?;
        Ok(res)
    }

    /// Queries for multiple rows using raw SQL.
    ///
    /// # Safety
    /// This method allows executing arbitrary SQL, bypassing type checks and
    /// tenant isolation. It should ONLY be used for test assertions.
    pub async fn query_all_check(
        &self,
        sql: &str,
    ) -> Result<Vec<sea_orm::QueryResult>, Box<dyn std::error::Error>> {
        let res = self
            .db
            .query_all(Statement::from_string(DbBackend::Postgres, sql.to_owned()))
            .await?;
        Ok(res)
    }
}
