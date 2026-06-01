\set ON_ERROR_STOP on

-- GA_08 production-plan gate for the canonical DB.
--
-- Local demo cardinality is intentionally smaller than production, so the
-- script disables sequential scans while planning. This does not prove row
-- counts; it proves each GA hot query has an index-backed path and fails if
-- the planner still needs a Seq Scan after indexes are preferred.

BEGIN;

SET LOCAL search_path = public, pg_catalog;
SET LOCAL enable_seqscan = off;

CREATE TEMP TABLE ga_plan_checks (
    check_name TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    details TEXT NOT NULL
) ON COMMIT DROP;

CREATE OR REPLACE FUNCTION pg_temp.ga_require_index(
    check_name TEXT,
    table_name TEXT,
    index_name TEXT
) RETURNS void
LANGUAGE plpgsql
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM pg_indexes
         WHERE schemaname = 'public'
           AND tablename = table_name
           AND indexname = index_name
    ) THEN
        RAISE EXCEPTION 'GA plan check % failed: required index %.% is missing',
            check_name, table_name, index_name;
    END IF;

    INSERT INTO ga_plan_checks(check_name, status, details)
    VALUES (check_name, 'PASS', format('required index present: %I', index_name));
END $$;

CREATE OR REPLACE FUNCTION pg_temp.ga_assert_no_seq_scan(
    check_name TEXT,
    query_sql TEXT
) RETURNS void
LANGUAGE plpgsql
AS $$
DECLARE
    plan_json JSONB;
    seq_details TEXT;
BEGIN
    EXECUTE 'EXPLAIN (FORMAT JSON) ' || query_sql INTO plan_json;

    WITH RECURSIVE nodes(node) AS (
        SELECT plan_json->0->'Plan'
        UNION ALL
        SELECT child
          FROM nodes,
               LATERAL jsonb_array_elements(COALESCE(node->'Plans', '[]'::jsonb)) AS child
    )
    SELECT string_agg(
               COALESCE(node->>'Schema', 'public')
               || '.'
               || COALESCE(node->>'Relation Name', '<unknown>')
               || ' via ' || COALESCE(node->>'Node Type', '<unknown>'),
               ', '
           )
      INTO seq_details
      FROM nodes
     WHERE node->>'Node Type' = 'Seq Scan'
       AND (
           COALESCE(node->>'Relation Name', '') IN (
               'output_distribution_cache',
               'run_length_distribution_cache'
           )
           OR EXISTS (
               SELECT 1
                 FROM pg_partition_tree('public.canonical_events'::regclass) AS pt
                 JOIN pg_class AS c
                   ON c.oid = pt.relid
                 JOIN pg_namespace AS n
                   ON n.oid = c.relnamespace
                WHERE n.nspname = COALESCE(node->>'Schema', 'public')
                  AND c.relname = COALESCE(node->>'Relation Name', '')
           )
       );

    IF seq_details IS NOT NULL THEN
        RAISE EXCEPTION 'GA plan check % failed: %', check_name, seq_details;
    END IF;

    INSERT INTO ga_plan_checks(check_name, status, details)
    VALUES (check_name, 'PASS', 'no Seq Scan over GA production tables');
END $$;

SELECT pg_temp.ga_require_index(
    'output_distribution_cache_hot_lookup_index',
    'output_distribution_cache',
    'output_distribution_cache_pkey'
);

SELECT pg_temp.ga_assert_no_seq_scan(
    'output_distribution_cache_hot_lookup',
    $SQL$
    SELECT p99_30d
      FROM output_distribution_cache
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'::uuid
       AND model = 'gpt-4o-mini'
       AND agent_id = 'agent-00'
       AND prompt_class = 'chat_short'
    $SQL$
);

SELECT pg_temp.ga_require_index(
    'run_length_distribution_cache_hot_lookup_index',
    'run_length_distribution_cache',
    'run_length_distribution_cache_pkey'
);

SELECT pg_temp.ga_assert_no_seq_scan(
    'run_length_distribution_cache_hot_lookup',
    $SQL$
    SELECT p95_steps_30d
      FROM run_length_distribution_cache
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'::uuid
       AND agent_id = 'agent-00'
    $SQL$
);

SELECT pg_temp.ga_require_index(
    'canonical_events_aggregator_bucket_index',
    'canonical_events',
    'canonical_events_aggregator_bucket_idx'
);

SELECT pg_temp.ga_assert_no_seq_scan(
    'canonical_events_output_distribution_aggregation',
    $SQL$
    SELECT model, agent_id, prompt_class, count(*) AS samples
      FROM canonical_events
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'::uuid
       AND recorded_month >= date_trunc('month', now() - interval '30 days')::date
       AND event_type = 'spendguard.audit.outcome'
       AND actual_output_tokens IS NOT NULL
     GROUP BY model, agent_id, prompt_class
    $SQL$
);

SELECT pg_temp.ga_require_index(
    'canonical_events_run_length_index',
    'canonical_events',
    'canonical_events_aggregator_run_length_idx'
);

SELECT pg_temp.ga_assert_no_seq_scan(
    'canonical_events_run_length_aggregation',
    $SQL$
    SELECT agent_id, run_id_mirror, count(*) AS decisions
      FROM canonical_events
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'::uuid
       AND recorded_month >= date_trunc('month', now() - interval '30 days')::date
       AND event_type = 'spendguard.audit.decision'
       AND run_id_mirror IS NOT NULL
     GROUP BY agent_id, run_id_mirror
    $SQL$
);

SELECT pg_temp.ga_require_index(
    'canonical_events_run_recovery_index',
    'canonical_events',
    'canonical_events_run_recovery_idx'
);

SELECT pg_temp.ga_assert_no_seq_scan(
    'canonical_events_run_recovery_lookup',
    $SQL$
    SELECT run_steps_completed_so_far,
           run_projection_at_decision_atomic::text AS run_projection_at_decision_atomic
      FROM canonical_events
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'::uuid
       AND event_type = 'spendguard.audit.decision'
       AND ingest_at >= clock_timestamp() - interval '30 minutes'
       AND recorded_month >= date_trunc('month', clock_timestamp() - interval '30 minutes')::date
       AND run_id_mirror = '11111111-1111-4111-8111-111111111111'::uuid
       AND agent_id = 'agent-00'
     ORDER BY producer_sequence DESC
     LIMIT 1
    $SQL$
);

SELECT pg_temp.ga_require_index(
    'canonical_events_decision_join_index',
    'canonical_events',
    'canonical_events_decision_idx'
);

SELECT pg_temp.ga_assert_no_seq_scan(
    'canonical_events_decision_outcome_join',
    $SQL$
    SELECT outcome.event_id
      FROM canonical_events AS outcome
      JOIN canonical_events AS decision
        ON decision.tenant_id = outcome.tenant_id
       AND decision.decision_id = outcome.decision_id
       AND decision.event_type = 'spendguard.audit.decision'
     WHERE outcome.tenant_id = '00000000-0000-4000-8000-000000000001'::uuid
       AND outcome.decision_id = '11111111-1111-4111-8111-111111111111'::uuid
       AND outcome.event_type = 'spendguard.audit.outcome'
    $SQL$
);

TABLE ga_plan_checks ORDER BY check_name;

COMMIT;
