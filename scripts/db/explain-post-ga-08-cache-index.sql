\set ON_ERROR_STOP on

-- POST_GA_08 / issue #166: planner evidence for keeping
-- output_distribution_cache_freshness_idx. The Strategy B hot lookup uses
-- output_distribution_cache_pkey; this script checks the separate
-- freshness/SLO query family that scans by computed_at.

BEGIN;

SET LOCAL search_path = public, pg_catalog;

CREATE TEMP TABLE post_ga_08_plan_checks (
    check_name TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    details TEXT NOT NULL
) ON COMMIT DROP;

CREATE OR REPLACE FUNCTION pg_temp.post_ga_08_assert_uses_index(
    check_name TEXT,
    query_sql TEXT,
    expected_index TEXT
) RETURNS void
LANGUAGE plpgsql
AS $$
DECLARE
    plan_json JSONB;
    used_indexes TEXT;
BEGIN
    EXECUTE 'EXPLAIN (FORMAT JSON) ' || query_sql INTO plan_json;

    WITH RECURSIVE nodes(node) AS (
        SELECT plan_json->0->'Plan'
        UNION ALL
        SELECT child
          FROM nodes,
               LATERAL jsonb_array_elements(COALESCE(node->'Plans', '[]'::jsonb)) AS child
    )
    SELECT string_agg(DISTINCT node->>'Index Name', ', ' ORDER BY node->>'Index Name')
      INTO used_indexes
      FROM nodes
     WHERE node ? 'Index Name';

    IF COALESCE(used_indexes, '') NOT LIKE '%' || expected_index || '%' THEN
        RAISE EXCEPTION 'POST_GA_08 plan check % failed: expected %, used indexes: %',
            check_name, expected_index, COALESCE(used_indexes, '<none>');
    END IF;

    INSERT INTO post_ga_08_plan_checks(check_name, status, details)
    VALUES (
        check_name,
        'PASS',
        format('uses %s; all indexes seen: %s', expected_index, used_indexes)
    );
END $$;

-- Representative enough for the normal-cost planner to prefer the
-- freshness index without disabling sequential scans: a small stale
-- subset among many fresh rows. This transaction is rolled back.
INSERT INTO output_distribution_cache (
    tenant_id, model, agent_id, prompt_class,
    sample_size_30d, computed_at, aggregation_version
)
SELECT
    format('00000000-0000-4000-8000-%012s', lpad(to_hex(g), 12, '0'))::uuid,
    'post-ga-08-model',
    'agent-' || g::text,
    'chat_short',
    30,
    CASE
        WHEN g <= 25 THEN clock_timestamp() - interval '3 hours' - make_interval(secs => g)
        ELSE clock_timestamp() - interval '5 minutes'
    END,
    'v1alpha1'
FROM generate_series(1, 50000) AS g;

ANALYZE output_distribution_cache;

SELECT pg_temp.post_ga_08_assert_uses_index(
    'output_distribution_cache_stale_range_scan',
    $SQL$
    SELECT count(*)
      FROM output_distribution_cache
     WHERE computed_at < now() - interval '2 hours'
    $SQL$,
    'output_distribution_cache_freshness_idx'
);

SELECT pg_temp.post_ga_08_assert_uses_index(
    'output_distribution_cache_max_computed_at_slo_probe',
    $SQL$
    SELECT max(computed_at)
      FROM output_distribution_cache
    $SQL$,
    'output_distribution_cache_freshness_idx'
);

TABLE post_ga_08_plan_checks ORDER BY check_name;

ROLLBACK;
