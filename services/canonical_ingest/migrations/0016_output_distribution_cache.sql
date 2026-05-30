-- ============================================================================
-- 0016_output_distribution_cache.sql — Stats Aggregator Strategy B cache.
--
-- Spec ancestors:
--   - docs/stats-aggregator-spec-v1alpha1.md §5 (authoritative DDL)
--   - docs/stats-aggregator-spec-v1alpha1.md §3.2 (bucket key)
--   - docs/stats-aggregator-spec-v1alpha1.md §9 (multi-tenant isolation)
--   - docs/slices/SLICE_06_output_predictor_a_b_stats_aggregator.md §5
--   - docs/output-predictor-service-spec-v1alpha1.md §4 (B cache consumer)
--
-- ## Why this table lives in canonical_ingest DB
--
-- Spec §3.1 — stats_aggregator reads `canonical_events` and writes the
-- pre-computed P50/P95/P99 cache. Co-locating the cache with the source
-- canonical_events table eliminates the cross-DB lookup the output_predictor
-- would otherwise incur. Reader: output_predictor connects directly to
-- canonical_ingest DB read-only (per spec §4.2 read-only connection pool).
--
-- ## Why NOT partitioned (spec §5.1)
--
-- Scale estimate per spec: 1000 tenants × 20 models × 50 agents × 7 classes
-- ≈ 7M rows max. ON CONFLICT UPSERT performs well at that scale; partition
-- pruning + cross-partition uniqueness costs outweigh the benefit.
-- Re-partition by `(tenant_id % 256)` if production scales beyond 50K
-- tenants (post-launch decision, tracked in spec §5.1).
--
-- Mutability: this is a derived/cache table — operators may TRUNCATE +
-- rebuild from canonical_events. No immutability trigger. No verify-chain
-- coverage (verify-chain audits canonical_events directly).
--
-- ## Privilege boundary
--
-- Mirrors 0005_immutability_triggers.sql roles:
--   - canonical_ingest_application_role: aggregation writer + reader (stats_aggregator)
--   - canonical_ingest_reader_role: read-only (output_predictor + calibration)
--
-- DELETE granted to application role for retention / rebuild cycles.
-- UPDATE granted because the aggregation cycle UPSERTs (spec §4.3).
--
-- ## Stylistic alignment
--
-- - psql autocommit per SLICE_01 R5 (each statement commits independently)
-- - SET LOCAL search_path = pg_catalog, pg_temp in DO blocks (CVE-2018-1058
--   hardening per SLICE_01 R5)
-- - TIMESTAMPTZ with TZ-explicit `+00` per SLICE_01 R5
-- - No down migration file per SLICE_03 R2 M3 convention; rollback via
--   `DROP TABLE output_distribution_cache CASCADE` (operator one-liner)
--
-- ## Multi-tenant isolation (spec §9)
--
-- Row-Level Security ON with FOR ALL policy: every SELECT and every
-- INSERT/UPDATE/DELETE requires the caller's session to have
-- `SET LOCAL app.current_tenant_id = '<uuid>'` before the query.
--
-- ### R2 B1 — writer path correction
--
-- R1 prior shape declared `FOR SELECT` + claimed `BYPASSRLS` for the
-- aggregation writer, but BYPASSRLS was never granted to the application
-- role (no GRANT BYPASSRLS in this migration; no role-level attribute in
-- 0005_immutability_triggers.sql). Under FORCE ROW LEVEL SECURITY the
-- writer's UPSERTs would be rejected with
-- "new row violates row-level security policy". R2 widens the policy to
-- FOR ALL and pairs it with the writer's SET LOCAL discipline (see
-- services/stats_aggregator/src/aggregation.rs::aggregate_output_distribution
-- — invokes `set_config('app.current_tenant_id', tenant, true)` before
-- every per-tenant UPSERT). Reader path (output_predictor cache.rs) sets
-- the same variable for SELECT. Same contract, no BYPASSRLS dependency.
-- ============================================================================

CREATE TABLE output_distribution_cache (
    -- Bucket key (per spec §3.2 + §5). Tenant UUID per the rest of the
    -- ledger schema convention (0003 budget_window_instances etc.). Model
    -- + agent_id + prompt_class are opaque application-defined identifiers;
    -- TEXT to remain bytes-clean for non-Latin model strings.
    tenant_id            UUID NOT NULL,
    model                TEXT NOT NULL,
    agent_id             TEXT NOT NULL,
    prompt_class         TEXT NOT NULL,

    -- 7-day rolling window (per spec §4.2 drift detection signal)
    p50_7d               REAL,
    p95_7d               REAL,
    p99_7d               REAL,
    mean_7d              REAL,
    stddev_7d            REAL,
    sample_size_7d       INTEGER CHECK (sample_size_7d IS NULL OR sample_size_7d >= 0),

    -- 30-day rolling window (per spec §4.2 Strategy B baseline)
    p50_30d              REAL,
    p95_30d              REAL,
    p99_30d              REAL,
    mean_30d             REAL,
    stddev_30d           REAL,
    sample_size_30d      INTEGER CHECK (sample_size_30d IS NULL OR sample_size_30d >= 0),

    -- Metadata. computed_at TZ-explicit +00 per SLICE_01 R5 convention.
    -- aggregation_version lets downstream consumers detect SQL aggregation
    -- recipe changes (spec §0.1 compatibility policy "aggregation_version
    -- column versioned").
    --
    -- R2 M17: no DEFAULT on computed_at — the aggregation writer ALWAYS
    -- supplies an explicit `now()`. Removing the default closes a foot-gun
    -- where an ad-hoc INSERT could land with `clock_timestamp()` from
    -- within a long-running transaction (the freshness gate downstream
    -- relies on the writer-supplied stamp being the real cycle time).
    computed_at          TIMESTAMPTZ NOT NULL,
    aggregation_version  TEXT NOT NULL DEFAULT 'v1alpha1',

    PRIMARY KEY (tenant_id, model, agent_id, prompt_class)
);

-- Freshness lookup: "find buckets where computed_at < cutoff" supports
-- both the SLO freshness alert and the rebuild trigger.
CREATE INDEX output_distribution_cache_freshness_idx
    ON output_distribution_cache (computed_at);

-- Tenant lookup index — Strategy B hot-path query (per spec §4.2).
-- Covers the most-common WHERE shape (tenant_id, model, agent_id,
-- prompt_class). PRIMARY KEY already covers (tenant_id, model,
-- agent_id, prompt_class) so this index is redundant for equality;
-- kept commented to document the access pattern — the optimiser
-- uses the PK btree directly.
-- CREATE INDEX output_distribution_cache_tenant_lookup_idx ...

-- ============================================================================
-- Row-Level Security (spec §9.1 mechanism 1).
--
-- Policy: `tenant_id = current_setting('app.current_tenant_id')::uuid`
-- means the calling session MUST have run
-- `SET LOCAL app.current_tenant_id = '<uuid>'` inside its transaction
-- before the SELECT. output_predictor::cache.rs does this per Predict
-- call. Operators querying ad-hoc via psql see 0 rows until they set
-- the session variable explicitly.
--
-- BYPASSRLS for the aggregation writer (stats_aggregator runs as the
-- application role with BYPASSRLS — see GRANT below). RLS only applies
-- to the reader path. Adversarial cross-tenant query injection (per
-- spec §9.2) is blocked by the policy + session variable contract.
-- ============================================================================

ALTER TABLE output_distribution_cache ENABLE ROW LEVEL SECURITY;
ALTER TABLE output_distribution_cache FORCE ROW LEVEL SECURITY;

-- R2 B1: FOR ALL policy. SELECT path enforces USING; INSERT/UPDATE/DELETE
-- additionally enforces WITH CHECK so a writer who forgets the SET LOCAL
-- still fails closed (cannot insert a row with mismatched tenant_id).
--
-- Use COALESCE to a deliberately-illegal sentinel so a missing session
-- variable produces a clean tenant-mismatch (returns 0 rows / rejects
-- WITH CHECK) rather than silently leaking every tenant's rows under a
-- NULL match. The sentinel '00000000-0000-0000-0000-000000000000' is
-- the nil UUID which never matches any production tenant_id (all
-- tenants mint UUIDv7 with timestamp > 0).
CREATE POLICY output_distribution_cache_tenant_isolation
    ON output_distribution_cache
    FOR ALL
    USING (
        tenant_id = COALESCE(
            NULLIF(current_setting('app.current_tenant_id', TRUE), ''),
            '00000000-0000-0000-0000-000000000000'
        )::uuid
    )
    WITH CHECK (
        tenant_id = COALESCE(
            NULLIF(current_setting('app.current_tenant_id', TRUE), ''),
            '00000000-0000-0000-0000-000000000000'
        )::uuid
    );

-- ============================================================================
-- Privilege boundary (mirror of 0005_immutability_triggers.sql convention).
--
-- canonical_ingest_application_role: stats_aggregator (writer) +
--   output_predictor pool (reader path goes through application role to
--   pick up RLS — reader role bypasses RLS by default which is wrong here).
-- canonical_ingest_reader_role: ad-hoc operator queries. RLS applies.
--
-- R2 M16: REVOKE SELECT FROM PUBLIC. Without it, any new connecting role
-- inherits PUBLIC's SELECT and can run cross-tenant probes through the
-- RLS policy. Belt-and-suspenders on top of RLS — defence in depth.
-- ============================================================================

REVOKE SELECT, INSERT, UPDATE, DELETE ON output_distribution_cache FROM PUBLIC;

GRANT SELECT, INSERT, UPDATE, DELETE
    ON output_distribution_cache
    TO canonical_ingest_application_role;

GRANT SELECT ON output_distribution_cache TO canonical_ingest_reader_role;

COMMENT ON TABLE output_distribution_cache IS
    'Strategy B per-(tenant, model, agent_id, prompt_class) P50/P95/P99 distribution cache per stats-aggregator-spec-v1alpha1.md §5. Populated hourly by services/stats_aggregator. Read by services/output_predictor on the Predict hot path. RLS FOR ALL policy enforces per-tenant isolation at both read AND write time; the stats_aggregator writer SETs app.current_tenant_id per tenant before each UPSERT (R2 B1). Mutable (UPSERT every cycle); no immutability trigger.';
COMMENT ON COLUMN output_distribution_cache.tenant_id IS
    'Tenant identifier (UUIDv7). Indexed via PRIMARY KEY for hot-path Predict lookup.';
COMMENT ON COLUMN output_distribution_cache.computed_at IS
    'Last successful aggregation cycle timestamp. Stale > 2h treated as cache miss by output_predictor per output-predictor-service-spec-v1alpha1.md §4.2.';
COMMENT ON COLUMN output_distribution_cache.aggregation_version IS
    'Aggregation recipe version; rows produced by older recipes can be invalidated by output_predictor when the spec version bumps.';

-- ============================================================================
-- DO-block smoke check: verify RLS is actually enabled and the policy
-- exists. CVE-2018-1058 hardening: SET LOCAL search_path so PostgreSQL
-- resolves built-in catalog names (pg_catalog.pg_class etc.) without
-- consulting a search_path that an adversary might have injected
-- (per SLICE_01 R5).
-- ============================================================================
DO $$
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    PERFORM 1 FROM pg_class
        WHERE relname = 'output_distribution_cache' AND relrowsecurity = TRUE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'output_distribution_cache RLS not enabled after migration';
    END IF;
    PERFORM 1 FROM pg_policy
        WHERE polname = 'output_distribution_cache_tenant_isolation';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'output_distribution_cache_tenant_isolation policy missing';
    END IF;
END $$;
