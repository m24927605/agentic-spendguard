-- Cost Advisor P0: cost_baselines table.
--
-- Spec: docs/specs/cost-advisor-spec.md §6 (Tier 2 baseline computation).
-- Default window = 28 days per §11.5 A4 (seasonality), not 7.
--
-- Applied against `spendguard_canonical`. Nightly `baseline_refresher`
-- worker (P2 deliverable) populates rows by issuing:
--
--   INSERT ... ON CONFLICT (tenant_id, agent_id, metric, window_days)
--     DO UPDATE SET ... (idempotent re-run)
--
-- For tenants with < window_days of data the refresher skips the row;
-- the outlier rule then emits an info-level "insufficient baseline"
-- finding rather than a quantified outlier (§11.5 A4).

CREATE TABLE cost_baselines (
    tenant_id           UUID NOT NULL,
    -- agent_id is sourced from the P0.5 sidecar enrichment workstream
    -- (see cost-advisor-p0-audit-report §5). Until that lands,
    -- baselines computed today carry an opaque placeholder for
    -- agent_id derived from `session_id` or `decision_id` per the
    -- runtime's degraded-mode fallback. Schema admits both.
    agent_id            TEXT NOT NULL,
    metric              TEXT NOT NULL CHECK (metric IN
                            ('cost_per_run',
                             'tokens_per_call',
                             'retries_per_run',
                             'idle_reservation_ratio',
                             'tool_calls_per_run')),
    window_days         INT NOT NULL CHECK (window_days IN (7, 28)),
    median              NUMERIC NOT NULL,
    p95                 NUMERIC NOT NULL,
    sample_count        INT NOT NULL CHECK (sample_count >= 0),
    computed_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),

    PRIMARY KEY (tenant_id, agent_id, metric, window_days)
);

CREATE INDEX cost_baselines_computed_at_idx
    ON cost_baselines (computed_at DESC);

COMMENT ON TABLE cost_baselines IS
    'Cost Advisor §6: per (tenant, agent, metric, window) rolling baselines refreshed nightly. Used by baseline-excess outlier rule (Tier 2). Default 28d window covers ~4 weekly cycles for seasonality robustness.';
