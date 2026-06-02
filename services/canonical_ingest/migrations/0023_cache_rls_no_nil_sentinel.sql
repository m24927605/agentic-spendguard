-- POST_GA_08 / issue #163: remove nil-UUID sentinel fallback from cache
-- RLS policies. A missing or empty app.current_tenant_id now casts to
-- NULL and the tenant_id comparison evaluates to false; no UUID value is
-- reserved as "impossible".

DROP POLICY IF EXISTS output_distribution_cache_tenant_isolation
    ON output_distribution_cache;

CREATE POLICY output_distribution_cache_tenant_isolation
    ON output_distribution_cache
    FOR ALL
    USING (
        tenant_id = NULLIF(current_setting('app.current_tenant_id', TRUE), '')::uuid
    )
    WITH CHECK (
        tenant_id = NULLIF(current_setting('app.current_tenant_id', TRUE), '')::uuid
    );

DROP POLICY IF EXISTS run_length_distribution_cache_tenant_isolation
    ON run_length_distribution_cache;

CREATE POLICY run_length_distribution_cache_tenant_isolation
    ON run_length_distribution_cache
    FOR ALL
    USING (
        tenant_id = NULLIF(current_setting('app.current_tenant_id', TRUE), '')::uuid
    )
    WITH CHECK (
        tenant_id = NULLIF(current_setting('app.current_tenant_id', TRUE), '')::uuid
    );

COMMENT ON POLICY output_distribution_cache_tenant_isolation
    ON output_distribution_cache IS
    'POST_GA_08: tenant_id must equal app.current_tenant_id; missing/empty setting becomes NULL and matches no row. No nil UUID sentinel.';

COMMENT ON POLICY run_length_distribution_cache_tenant_isolation
    ON run_length_distribution_cache IS
    'POST_GA_08: tenant_id must equal app.current_tenant_id; missing/empty setting becomes NULL and matches no row. No nil UUID sentinel.';

COMMENT ON INDEX output_distribution_cache_freshness_idx IS
    'POST_GA_08 #166: retained for freshness range scans and max(computed_at) SLO probes; hot lookup continues to use output_distribution_cache_pkey.';

DO $$
DECLARE
    policy_sql text;
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;

    SELECT pg_get_expr(polqual, polrelid) || ' ' || pg_get_expr(polwithcheck, polrelid)
      INTO policy_sql
      FROM pg_policy
     WHERE polname = 'output_distribution_cache_tenant_isolation';
    IF policy_sql IS NULL THEN
        RAISE EXCEPTION 'output_distribution_cache_tenant_isolation policy missing';
    END IF;
    IF policy_sql LIKE '%00000000-0000-0000-0000-000000000000%' THEN
        RAISE EXCEPTION 'output_distribution_cache_tenant_isolation still uses nil UUID sentinel';
    END IF;

    SELECT pg_get_expr(polqual, polrelid) || ' ' || pg_get_expr(polwithcheck, polrelid)
      INTO policy_sql
      FROM pg_policy
     WHERE polname = 'run_length_distribution_cache_tenant_isolation';
    IF policy_sql IS NULL THEN
        RAISE EXCEPTION 'run_length_distribution_cache_tenant_isolation policy missing';
    END IF;
    IF policy_sql LIKE '%00000000-0000-0000-0000-000000000000%' THEN
        RAISE EXCEPTION 'run_length_distribution_cache_tenant_isolation still uses nil UUID sentinel';
    END IF;
END $$;
