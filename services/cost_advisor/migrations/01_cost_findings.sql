-- Cost Advisor P0: cost_findings table.
--
-- Spec: docs/specs/cost-advisor-spec.md §4.1 (table) + §4.0
-- (FindingEvidence shape carried in `evidence` JSONB).
--
-- Applied against the `spendguard_canonical` database alongside
-- `canonical_events`. Wired in via
-- `deploy/demo/init/migrations/21_apply_cost_advisor_migrations.sh`.
--
-- Partitioning per §11.5 A7 (storage strategy): monthly partitions on
-- `detected_at`. Hot-tier (postgres) keeps the last 90 days; older
-- partitions archive to S3 as Parquet + DETACH. The retention sweeper
-- (P1 work; tenant_data_policy.cost_findings_retention_days_open /
-- _resolved drive policy) DELETEs rows past the per-tenant window.
-- Per-tenant DELETEs are allowed here (no immutability trigger) —
-- cost_findings are derived artifacts, not the audit chain.

CREATE TABLE cost_findings (
    finding_id          UUID NOT NULL,                  -- UUID v7 from rule emitter
    fingerprint         CHAR(64) NOT NULL,              -- SHA-256 hex (spec §11.5 A1)
    tenant_id           UUID NOT NULL,
    detected_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    rule_id             TEXT NOT NULL,                  -- e.g. 'idle_reservation_rate_v1'
    rule_version        INT NOT NULL DEFAULT 1,
    category            TEXT NOT NULL CHECK (category IN
                            ('detected_waste', 'optimization_hypothesis')),
    severity            TEXT NOT NULL CHECK (severity IN
                            ('critical', 'warn', 'info')),
    confidence          NUMERIC(3,2) NOT NULL CHECK (confidence BETWEEN 0.00 AND 1.00),
    -- Scope (at most one of these set; NULL for tenant_global scope).
    agent_id            TEXT,
    run_id              TEXT,
    contract_bundle_id  TEXT,
    -- Evidence: serialized FindingEvidence proto (proto3 → JSON well-known
    -- mapping). Columns above are denormalized projections that satisfy
    -- the §4.0 schema's stable-shape requirement for dashboard / CLI /
    -- dedup consumers.
    evidence            JSONB NOT NULL,
    -- Quantified impact. NULL for unquantifiable hypothesis findings.
    estimated_waste_micros_usd  BIGINT,
    sample_decision_ids         UUID[] NOT NULL,        -- pointers into canonical_events
    -- Lifecycle.
    status              TEXT NOT NULL DEFAULT 'open' CHECK (status IN
                            ('open', 'dismissed', 'fixed', 'superseded')),
    superseded_by       UUID,                            -- set when §5.1.1 dedup phase suppresses
    feedback            TEXT,                            -- '👍' / '👎' / NULL
    -- Optional Tier-3 narrative (lazy-populated by P3 narrative wrapper).
    narrative_md        TEXT,
    narrative_model     TEXT,                            -- e.g. 'gpt-4o-mini-2024-07-18'
    narrative_cost_usd  NUMERIC(10,6),
    narrative_at        TIMESTAMPTZ,
    -- Audit.
    created_at          TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),

    PRIMARY KEY (tenant_id, detected_at, finding_id)
)
PARTITION BY RANGE (detected_at);

-- Initial monthly partitions covering "now ± a few months" so the demo
-- + early prod can INSERT without partition-not-found errors. CD job
-- creates forward partitions as time advances.
CREATE TABLE cost_findings_2026_05 PARTITION OF cost_findings
    FOR VALUES FROM ('2026-05-01') TO ('2026-06-01');
CREATE TABLE cost_findings_2026_06 PARTITION OF cost_findings
    FOR VALUES FROM ('2026-06-01') TO ('2026-07-01');
CREATE TABLE cost_findings_2026_07 PARTITION OF cost_findings
    FOR VALUES FROM ('2026-07-01') TO ('2026-08-01');
CREATE TABLE cost_findings_default PARTITION OF cost_findings DEFAULT;

-- Idempotency: rule re-runs UPSERT on (tenant_id, fingerprint).
-- Partial unique on the partitioned table needs to be expressed as a
-- non-partitioned mirror because Postgres requires UNIQUE constraints
-- on partitioned tables to include the partition key.
CREATE TABLE cost_findings_fingerprint_keys (
    tenant_id   UUID NOT NULL,
    fingerprint CHAR(64) NOT NULL,
    finding_id  UUID NOT NULL,
    PRIMARY KEY (tenant_id, fingerprint)
);

CREATE INDEX cost_findings_tenant_detected_idx
    ON cost_findings (tenant_id, detected_at DESC);

CREATE INDEX cost_findings_tenant_open_idx
    ON cost_findings (tenant_id, severity)
    WHERE status = 'open';

CREATE INDEX cost_findings_rule_id_idx
    ON cost_findings (rule_id, rule_version);

-- Touch trigger keeps updated_at in lockstep with row mutation
-- (lifecycle transitions, narrative population).
CREATE OR REPLACE FUNCTION cost_findings_touch()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = clock_timestamp();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER cost_findings_touch_trg
    BEFORE UPDATE ON cost_findings
    FOR EACH ROW EXECUTE FUNCTION cost_findings_touch();

COMMENT ON TABLE cost_findings IS
    'Cost Advisor §4.1: derived findings emitted by rules. Idempotent UPSERT keyed by (tenant_id, fingerprint). Lifecycle: open → dismissed | fixed | superseded. Retention driven by tenant_data_policy.cost_findings_retention_days_* (ledger DB) + retention_sweeper (P1 sweep kind).';
