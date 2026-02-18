use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // 1. Create Schema
        db.execute_unprepared("CREATE SCHEMA IF NOT EXISTS flexi")
            .await?;

        // 2. Create Nonce Table (Partitioned by day)
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.flexi_nonce (
                nonce TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                PRIMARY KEY (nonce, created_at)
            ) PARTITION BY RANGE (created_at);
            "#,
        )
        .await?;

        // Initial Partitions (Today + Next day)
        // Uses a DEFAULT partition for initial setup. In production, replace with
        // pg_partman or a scheduled partition-maintenance job and a retention policy (~2 days).
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
        )
        .await?;

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
            $$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = flexi, pg_catalog, pg_temp;

            REVOKE ALL ON FUNCTION flexi.check_nonce_uniqueness() FROM PUBLIC;

            DROP TRIGGER IF EXISTS nonce_uniqueness_trigger ON flexi.flexi_nonce;
            CREATE TRIGGER nonce_uniqueness_trigger
            BEFORE INSERT ON flexi.flexi_nonce
            FOR EACH ROW EXECUTE FUNCTION flexi.check_nonce_uniqueness();
            "#,
        )
        .await?;

        // 4. Create Authorize Function
        db.execute_unprepared("CREATE EXTENSION IF NOT EXISTS pgcrypto SCHEMA flexi")
            .await?;
        db.execute_unprepared(
            r#"
            CREATE OR REPLACE FUNCTION flexi.authorize_tenant(token_val text) RETURNS void AS $$
            DECLARE
                parts text[];
                ver text;
                kid text;
                ts_str text;
                nonce_val text;
                tenant_id_val text;
                sig text;
                computed_sig text;
                ts bigint;
                now_ts bigint;
                secret text;
            BEGIN
                -- 1. Input Validation
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
                secret := current_setting('flexi.hmac_secret', true);
                
                -- CRITICAL: Fail closed if secret is not set
                IF secret IS NULL OR secret = '' THEN
                    RAISE EXCEPTION 'HMAC secret not set';
                END IF;
                
                -- GATE: Check for dev_mode GUC or specific mock allowed
                IF current_setting('flexi.dev_mode', true) = 'on' AND sig = 'mock_sig' THEN
                    -- Allow mock in dev mode
                ELSE
                    -- Real HMAC verification (fail-closed if not dev and not verified)
                    computed_sig := encode(hmac(ver || ':' || kid || ':' || ts_str || ':' || nonce_val || ':' || tenant_id_val, secret, 'sha256'), 'hex');
                    IF sig IS DISTINCT FROM computed_sig THEN
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

                -- 7. Set Context Integrity Signature (Anti-Tampering)
                -- We sign the tenant_id with the internal secret so authorized_tenant_id() can verify
                -- that the context was set by this authorized function and not by a raw SET command.
                PERFORM set_config('flexi.ctx_sig', encode(hmac(tenant_id_val, secret, 'sha256'), 'hex'), true);
            END;
            $$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = flexi, pg_catalog, pg_temp;

            -- Revoke public access
            REVOKE ALL ON FUNCTION flexi.authorize_tenant(text) FROM PUBLIC;
            
            -- Grant EXECUTE to 'flexi' role only if it exists
            DO $$
            BEGIN
                IF EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi') THEN
                    GRANT EXECUTE ON FUNCTION flexi.authorize_tenant(text) TO flexi;
                END IF;
            END $$;
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
            DECLARE
                tid text;
                sig text;
                secret text;
                computed_sig text;
            BEGIN
                tid := current_setting('flexi.current_tenant', true);
                if tid IS NULL OR tid = '' THEN RETURN NULL; END IF;

                sig := current_setting('flexi.ctx_sig', true);
                secret := current_setting('flexi.hmac_secret', true);

                -- CRITICAL: Fail closed if secret is not set
                IF secret IS NULL OR secret = '' THEN
                    RAISE EXCEPTION 'HMAC secret not set';
                END IF;

                -- Verify integrity: Re-compute HMAC and compare
                -- If someone did SET flexi.current_tenant = 'x', the sig won't match.
                computed_sig := encode(hmac(tid, secret, 'sha256'), 'hex');
                IF sig IS DISTINCT FROM computed_sig THEN
                    RAISE EXCEPTION 'Tenant context integrity check failed';
                END IF;

                RETURN tid;
            END;
            $$ LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = flexi, pg_catalog, pg_temp;

            -- Revoke public access for defense-in-depth
            REVOKE ALL ON FUNCTION flexi.authorized_tenant_id() FROM PUBLIC;
            
            -- Grant EXECUTE to 'flexi' role only if it exists
            DO $$
            BEGIN
                IF EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'flexi') THEN
                    GRANT EXECUTE ON FUNCTION flexi.authorized_tenant_id() TO flexi;
                END IF;
            END $$;
            "#
        ).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP SCHEMA IF EXISTS flexi CASCADE")
            .await?;
        Ok(())
    }
}
