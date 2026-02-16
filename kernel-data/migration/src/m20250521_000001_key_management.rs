use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // 1. Create Key Record Table
        db.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS flexi.key_record (
                kid TEXT PRIMARY KEY,
                key_type TEXT NOT NULL CHECK (key_type IN ('hmac', 'paseto_public', 'paseto_private')),
                algorithm TEXT NOT NULL,
                secret_bytes BYTEA,
                public_bytes BYTEA,
                state TEXT NOT NULL CHECK (state IN ('active', 'next', 'retired', 'revoked')),
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                activated_at TIMESTAMPTZ,
                retired_at TIMESTAMPTZ,
                revoked_at TIMESTAMPTZ,
                expires_at TIMESTAMPTZ
            );

            CREATE INDEX idx_key_record_state ON flexi.key_record (state);
            CREATE INDEX idx_key_record_type_state ON flexi.key_record (key_type, state);
            "#
        ).await?;

        // 2. Update Authorize Function to use Key Record
        // Instead of relying on GUC 'flexi.hmac_secret' for the external token, lookup in key_record.
        // We still use 'flexi.hmac_secret' for the internal context integrity (ctx_sig).
        db.execute_unprepared(
            r#"
            CREATE OR REPLACE FUNCTION flexi.authorize_tenant(token_val text) RETURNS void AS $$
            DECLARE
                parts text[];
                ver text;
                kid_val text;
                ts_str text;
                nonce_val text;
                tenant_id_val text;
                sig text;
                computed_sig text;
                ts bigint;
                now_ts bigint;
                secret_key bytea;
                internal_secret text;
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
                kid_val := parts[2];
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

                -- 4. Verify Signature (HMAC-SHA256) using Key from Table

                -- GATE: Check for dev_mode GUC or specific mock allowed
                IF current_setting('flexi.dev_mode', true) = 'on' AND sig = 'mock_sig' THEN
                    -- Allow mock in dev mode
                ELSE
                    -- Lookup key
                    SELECT secret_bytes INTO secret_key
                    FROM flexi.key_record
                    WHERE kid = kid_val
                      AND key_type = 'hmac'
                      AND state IN ('active', 'next', 'retired');

                    IF secret_key IS NULL THEN
                        RAISE EXCEPTION 'Invalid or expired key ID: %', kid_val;
                    END IF;

                    -- Real HMAC verification (fail-closed if not verified)
                    -- Note: pgcrypto hmac takes bytea key
                    computed_sig := encode(hmac(ver || ':' || kid_val || ':' || ts_str || ':' || nonce_val || ':' || tenant_id_val, secret_key, 'sha256'), 'hex');
                    IF sig IS DISTINCT FROM computed_sig THEN
                         RAISE EXCEPTION 'Invalid signature';
                    END IF;
                END IF;

                -- 5. Check Nonce (Consumption)
                BEGIN
                    INSERT INTO flexi.flexi_nonce (nonce, created_at)
                    VALUES (nonce_val, to_timestamp(ts::double precision));
                EXCEPTION WHEN unique_violation THEN
                    RAISE EXCEPTION 'Nonce already used';
                END;

                -- 6. Set Context
                PERFORM set_config('flexi.current_tenant', tenant_id_val, true);

                -- 7. Set Context Integrity Signature (Anti-Tampering)
                -- We verify the token using the external key (Rotated).
                -- We sign the context using the internal secret (Static GUC).
                internal_secret := current_setting('flexi.hmac_secret', true);

                -- CRITICAL: Fail closed if internal secret is not set
                IF internal_secret IS NULL OR internal_secret = '' THEN
                    RAISE EXCEPTION 'Internal HMAC secret not set';
                END IF;

                PERFORM set_config('flexi.ctx_sig', encode(hmac(tenant_id_val, internal_secret, 'sha256'), 'hex'), true);
            END;
            $$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = flexi, pg_catalog, pg_temp;
            "#
        ).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Revert authorize_tenant to original version (using GUC only)
        // Note: Ideally we should restore the exact previous version, but for simplicity here we just drop the table.
        // If we drop the table, the function will fail if it references it.
        // So we should revert the function first.

        // Re-create original authorize_tenant
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
                -- Original Logic (Simplified for rollback)
                IF token_val IS NULL OR token_val = '' THEN RAISE EXCEPTION 'Missing token'; END IF;
                parts := string_to_array(token_val, ':');
                IF array_length(parts, 1) != 6 THEN RAISE EXCEPTION 'Invalid token'; END IF;
                ver := parts[1]; kid := parts[2]; ts_str := parts[3]; nonce_val := parts[4]; tenant_id_val := parts[5]; sig := parts[6];

                ts := ts_str::bigint;
                now_ts := extract(epoch from now())::bigint;
                IF ts < (now_ts - 30) OR ts > (now_ts + 30) THEN RAISE EXCEPTION 'Expired'; END IF;

                secret := current_setting('flexi.hmac_secret', true);
                IF secret IS NULL OR secret = '' THEN RAISE EXCEPTION 'Secret not set'; END IF;

                IF current_setting('flexi.dev_mode', true) = 'on' AND sig = 'mock_sig' THEN
                ELSE
                    computed_sig := encode(hmac(ver || ':' || kid || ':' || ts_str || ':' || nonce_val || ':' || tenant_id_val, secret, 'sha256'), 'hex');
                    IF sig IS DISTINCT FROM computed_sig THEN RAISE EXCEPTION 'Invalid signature'; END IF;
                END IF;

                BEGIN
                    INSERT INTO flexi.flexi_nonce (nonce, created_at) VALUES (nonce_val, to_timestamp(ts::double precision));
                EXCEPTION WHEN unique_violation THEN RAISE EXCEPTION 'Nonce used'; END;

                PERFORM set_config('flexi.current_tenant', tenant_id_val, true);
                PERFORM set_config('flexi.ctx_sig', encode(hmac(tenant_id_val, secret, 'sha256'), 'hex'), true);
            END;
            $$ LANGUAGE plpgsql SECURITY DEFINER SET search_path = flexi, pg_catalog, pg_temp;
            "#
        ).await?;

        db.execute_unprepared("DROP TABLE IF EXISTS flexi.key_record").await?;

        Ok(())
    }
}
