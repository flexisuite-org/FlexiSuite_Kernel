use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // 1. Create Schema
        db.execute_unprepared("CREATE SCHEMA IF NOT EXISTS flexi").await?;

        // 2. Create Nonce Table (Partitioned by day)
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.flexi_nonce (
                nonce TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (nonce, created_at)
            ) PARTITION BY RANGE (created_at);
            "#,
        ).await?;

        // Initial Partitions (Today + Next day)
        // Ideally this should be managed by pg_partman, but for initiation we create default.
        // For simplicity in MVP/Dev, we can start with a default partition or just let it fail if not set up?
        // No, let's create a default partition that covers everything for now to pass tests.
        // In Prod, maintenance script handles this.
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS flexi.flexi_nonce_default PARTITION OF flexi.flexi_nonce DEFAULT;"
        ).await?;

        // 2b. Create Nonce Uniqueness Guard Table (Non-partitioned)
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.nonce_uniqueness (
                nonce TEXT PRIMARY KEY,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );
            "#,
        ).await?;

        // 3. Create Uniqueness Trigger (Global uniqueness via nonce_uniqueness table)
        db.execute_unprepared(
            r#"
            CREATE OR REPLACE FUNCTION flexi.check_nonce_uniqueness() RETURNS TRIGGER AS $$
            BEGIN
                -- Atomic uniqueness check by inserting into the non-partitioned uniqueness table
                -- If it fails, the whole transaction (including partition insert) fails
                INSERT INTO flexi.nonce_uniqueness (nonce, created_at)
                VALUES (NEW.nonce, NEW.created_at);
                RETURN NEW;
            EXCEPTION WHEN unique_violation THEN
                RAISE EXCEPTION 'Nonce already used' USING ERRCODE = 'unique_violation';
            END;
            $$ LANGUAGE plpgsql SECURITY DEFINER;

            CREATE TRIGGER nonce_uniqueness_trigger
            BEFORE INSERT ON flexi.flexi_nonce
            FOR EACH ROW EXECUTE FUNCTION flexi.check_nonce_uniqueness();
            "#
        ).await?;

        // 4. Create Authorize Function
        db.execute_unprepared("CREATE EXTENSION IF NOT EXISTS pgcrypto").await?;
        db.execute_unprepared(
            r#"
            CREATE OR REPLACE FUNCTION flexi.authorize_tenant() RETURNS void AS $$
            DECLARE
                token_val text;
                parts text[];
                ver text;
                kid text;
                ts_str text;
                nonce_val text;
                tenant_id_val text;
                sig text;
                ts bigint;
                now_ts bigint;
            BEGIN
                -- 1. Get Token from GUC
                token_val := current_setting('flexi.tenant_token', true);
                IF token_val IS NULL OR token_val = '' THEN
                    RAISE EXCEPTION 'Missing or empty tenant token';
                END IF;

                -- 2. Parse Token (v2:kid:ts:nonce:tenant_id:sig)
                parts := string_to_array(token_val, ':');
                IF array_length(parts, 1) != 6 THEN
                    RAISE EXCEPTION 'Invalid token format';
                END IF;

                ver := parts[1];
                kid := parts[2];
                ts_str := parts[3];
                nonce_val := parts[4];
                tenant_id_val := parts[5];
                sig := parts[6];

                IF ver != 'v2' THEN
                    RAISE EXCEPTION 'Unsupported token version: %', ver;
                END IF;

                -- 3. Validate Timestamp (±30s)
                ts := ts_str::bigint;
                now_ts := extract(epoch from now())::bigint;
                IF ts < (now_ts - 30) OR ts > (now_ts + 30) THEN
                    RAISE EXCEPTION 'Token timestamp expired or future (skew > 30s)';
                END IF;

                -- 4. Verify Signature (HMAC-SHA256)
                -- In production, load secret from a secure source. 
                -- GATE: Check for dev_mode GUC or specific mock allowed
                IF current_setting('flexi.dev_mode', true) = 'on' AND sig = 'mock_sig' THEN
                    -- Allow mock in dev mode
                ELSE
                    -- Real HMAC verification (Implementation would use extension if needed,
                    -- here we fail-closed if not dev and not verified)
                    -- For MDP, we require HMAC or fail if not in dev.
                    IF sig != encode(hmac(ver || ':' || kid || ':' || ts_str || ':' || nonce_val || ':' || tenant_id_val, current_setting('flexi.hmac_secret'), 'sha256'), 'hex') THEN
                         RAISE EXCEPTION 'Invalid signature';
                    END IF;
                END IF;

                -- 5. Check Nonce (Consumption)
                -- The trigger 'nonce_uniqueness_trigger' ensures global uniqueness of 'nonce'
                -- even if 'created_at' (partition key) is different.
                BEGIN
                    INSERT INTO flexi.flexi_nonce (nonce, created_at) 
                    VALUES (nonce_val, to_timestamp(ts::double precision));
                EXCEPTION WHEN unique_violation THEN
                    RAISE EXCEPTION 'Nonce already used';
                END;

                -- 6. Set Context
                PERFORM set_config('flexi.current_tenant', tenant_id_val, true);
            END;
            $$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = flexi, pg_catalog, pg_temp;

            -- Revoke public access
            REVOKE ALL ON FUNCTION flexi.authorize_tenant() FROM PUBLIC;
            "#
        ).await?;

        // 4. Create authorized_tenant_id() helper for RLS
        // NOTE: This helper is used by RLS policies to identify the current tenant.
        // It relies on 'flexi.current_tenant' GUC which is set by flexi.authorize_tenant().
        // In production, partition management (e.g., pg_partman) should be used for flexi.flexi_nonce.
        // Since nonces are only valid within a ±30s window, a retention policy of 1-2 days is recommended.
        db.execute_unprepared(
            r#"
            CREATE OR REPLACE FUNCTION flexi.authorized_tenant_id() RETURNS text AS $$
            BEGIN
                RETURN current_setting('flexi.current_tenant', true);
            END;
            $$ LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = flexi, pg_catalog, pg_temp;

            -- Revoke public access for defense-in-depth
            REVOKE ALL ON FUNCTION flexi.authorized_tenant_id() FROM PUBLIC;
            "#
        ).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP SCHEMA IF EXISTS flexi CASCADE").await?;
        Ok(())
    }
}
