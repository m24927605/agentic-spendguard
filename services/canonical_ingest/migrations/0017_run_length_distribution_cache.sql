-- ============================================================================
-- 0017_run_length_distribution_cache.sql — Stats Aggregator (tenant, agent_id)
-- run-length distribution cache.
--
-- Spec ancestors:
--   - docs/stats-aggregator-spec-v1alpha1.md §6 (authoritative DDL)
--   - docs/slices/SLICE_06_output_predictor_a_b_stats_aggregator.md §5
--   - docs/run-cost-projector-spec-v1alpha1.md Signal 1 (downstream consumer)
--
-- ## Purpose
--
-- Per spec §6 the stats_aggregator computes per-(tenant, agent_id) historical
-- P95 run length (steps per run) on the same hourly cadence as the output
-- distribution cache. The run_cost_projector (SLICE_09) consumes this for
-- Signal 1 cost projection. SLICE_06 lands the schema; SLICE_09 wires the
-- consumer.
--
-- ## Why a separate table
--
-- Bucket key differs from output_distribution_cache (no model/prompt_class
-- breakdown). Different read pattern. Different aggregation source query.
-- Co-located in canonical_ingest DB for the same locality reason as 0016.
--
-- ## Privilege boundary + RLS
--
-- Same shape as 0016. RLS enforced; canonical_ingest_application_role gets
-- INSERT/UPDATE/DELETE/SELECT; canonical_ingest_reader_role gets SELECT.
--
-- ## Stylistic alignment
--
-- - psql autocommit per SLICE_01 R5
-- - SET LOCAL search_path = pg_catalog, pg_temp in DO blocks
-- - TIMESTAMPTZ with TZ-explicit +00 per SLICE_01 R5
-- - No down migration file per SLICE_03 R2 M3 convention
-- ============================================================================

CREATE TABLE run_length_distribution_cache (
    -- Bucket key (per spec §6). (tenant_id, agent_id) tuple — no model
    -- or prompt_class because run length is per-agent semantic property
    -- not per-call semantic property.
    tenant_id            UUID NOT NULL,
    agent_id             TEXT NOT NULL,

    -- 30-day rolling window only (spec §6 — drift detection on run length
    -- is SLICE-extra; 7d window not needed for v1alpha1).
    p50_steps_30d        REAL,
    p95_steps_30d        REAL,
    p99_steps_30d        REAL,
    mean_steps_30d       REAL,
    stddev_steps_30d     REAL,
    sample_size_30d      INTEGER CHECK (sample_size_30d IS NULL OR sample_size_30d >= 0),

    -- Metadata. TZ-explicit +00 per SLICE_01 R5.
    computed_at          TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    aggregation_version  TEXT NOT NULL DEFAULT 'v1alpha1',

    PRIMARY KEY (tenant_id, agent_id)
);

CREATE INDEX run_length_distribution_cache_freshness_idx
    ON run_length_distribution_cache (computed_at);

-- ============================================================================
-- Row-Level Security (same contract as 0016).
-- ============================================================================

ALTER TABLE run_length_distribution_cache ENABLE ROW LEVEL SECURITY;
ALTER TABLE run_length_distribution_cache FORCE ROW LEVEL SECURITY;

CREATE POLICY run_length_distribution_cache_tenant_isolation
    ON run_length_distribution_cache
    FOR SELECT
    USING (
        tenant_id = COALESCE(
            NULLIF(current_setting('app.current_tenant_id', TRUE), ''),
            '00000000-0000-0000-0000-000000000000'
        )::uuid
    );

-- ============================================================================
-- Privilege boundary.
-- ============================================================================

REVOKE INSERT, UPDATE, DELETE ON run_length_distribution_cache FROM PUBLIC;

GRANT SELECT, INSERT, UPDATE, DELETE
    ON run_length_distribution_cache
    TO canonical_ingest_application_role;

GRANT SELECT ON run_length_distribution_cache TO canonical_ingest_reader_role;

COMMENT ON TABLE run_length_distribution_cache IS
    'Per-(tenant, agent_id) run-length (steps per run) distribution cache per stats-aggregator-spec-v1alpha1.md §6. SLICE_06 ships schema; SLICE_09 (run_cost_projector) wires the consumer. RLS enforces per-tenant read isolation.';
COMMENT ON COLUMN run_length_distribution_cache.p95_steps_30d IS
    'P95 of `count(*) GROUP BY run_id` over decisions in the last 30 days. Consumed by run_cost_projector Signal 1 (SLICE_09).';
COMMENT ON COLUMN run_length_distribution_cache.computed_at IS
    'Last successful aggregation cycle timestamp.';

-- ============================================================================
-- DO-block smoke check.
-- ============================================================================
DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    PERFORM 1 FROM pg_class
        WHERE relname = 'run_length_distribution_cache' AND relrowsecurity = TRUE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'run_length_distribution_cache RLS not enabled after migration';
    END IF;
    PERFORM 1 FROM pg_policy
        WHERE polname = 'run_length_distribution_cache_tenant_isolation';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'run_length_distribution_cache_tenant_isolation policy missing';
    END IF;
END $$;
