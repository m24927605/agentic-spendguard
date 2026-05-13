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
-- + early prod can INSERT without partition-not-found errors. Beyond
-- August 2026 forward partitions are created by a P1 background
-- worker that calls cost_findings_ensure_next_month_partition()
-- (defined below) on a daily cron.
CREATE TABLE cost_findings_2026_05 PARTITION OF cost_findings
    FOR VALUES FROM ('2026-05-01') TO ('2026-06-01');
CREATE TABLE cost_findings_2026_06 PARTITION OF cost_findings
    FOR VALUES FROM ('2026-06-01') TO ('2026-07-01');
CREATE TABLE cost_findings_2026_07 PARTITION OF cost_findings
    FOR VALUES FROM ('2026-07-01') TO ('2026-08-01');

-- DEFAULT partition operates as a safety net so a delayed forward-
-- partition CD job never blocks ingest. Codex r5 P1-6 flagged that
-- this creates a trap: once a row dated e.g. 2026-08-15 lands in
-- DEFAULT, later `CREATE TABLE cost_findings_2026_08 PARTITION OF
-- ... FOR VALUES FROM ('2026-08-01') TO ('2026-09-01')` fails with
-- "updated partition constraint for default partition would be
-- violated by some row". Mitigation:
--   1. A background worker MUST keep forward partitions ahead of the
--      wallclock by at least one month (see the SP below). This is
--      P1 wiring; P0 doc'd as a known follow-up.
--   2. If DEFAULT is ever non-empty, operators run
--      cost_findings_drain_default_into_new_partition() (below) which
--      copies the rows into the explicit partition before creating it.
--   3. A monitoring alert fires when cost_findings_default has any
--      rows for more than 24h (defined in deploy/observability/
--      prometheus-rules.yaml; P1 wires this).
CREATE TABLE cost_findings_default PARTITION OF cost_findings DEFAULT;

-- Helper: idempotent creation of the next month's partition.
-- Called daily by the P1 partition-management worker. Returns the
-- newly-created partition name, or NULL if it already exists.
-- The worker uses the result to log "created X" without false-positive
-- noise on subsequent runs.
--
-- Drain semantics: if rows have already landed in DEFAULT that belong
-- to the target month (because the CD worker fell behind), they are
-- moved into the new partition, NOT discarded. The whole operation
-- runs under ACCESS EXCLUSIVE on cost_findings — a few milliseconds
-- for the rare backfill case — so no concurrent INSERTs can race the
-- DELETE→CREATE→INSERT sequence.
CREATE OR REPLACE FUNCTION cost_findings_ensure_next_month_partition()
    RETURNS TEXT LANGUAGE plpgsql AS $$
DECLARE
    v_next_start DATE;
    v_next_end   DATE;
    v_part_name  TEXT;
    v_drained    INT;
BEGIN
    v_next_start := date_trunc('month', now() + INTERVAL '1 month')::DATE;
    v_next_end   := v_next_start + INTERVAL '1 month';
    v_part_name  := 'cost_findings_' || to_char(v_next_start, 'YYYY_MM');

    IF to_regclass(v_part_name) IS NOT NULL THEN
        RETURN NULL;
    END IF;

    LOCK TABLE cost_findings IN ACCESS EXCLUSIVE MODE;

    -- Stage rows that need to be moved out of DEFAULT.
    CREATE TEMP TABLE _drain ON COMMIT DROP AS
      SELECT * FROM cost_findings_default
       WHERE detected_at >= v_next_start AND detected_at < v_next_end;

    GET DIAGNOSTICS v_drained = ROW_COUNT;

    IF v_drained > 0 THEN
        DELETE FROM cost_findings_default
         WHERE detected_at >= v_next_start AND detected_at < v_next_end;
    END IF;

    EXECUTE format(
        'CREATE TABLE %I PARTITION OF cost_findings FOR VALUES FROM (%L) TO (%L)',
        v_part_name, v_next_start, v_next_end
    );

    IF v_drained > 0 THEN
        -- Re-insert via the parent so the new partition (and any
        -- relevant indexes) receive the rows.
        INSERT INTO cost_findings SELECT * FROM _drain;
    END IF;

    RETURN v_part_name;
END;
$$;

COMMENT ON FUNCTION cost_findings_ensure_next_month_partition IS
    'Cost Advisor P0: idempotent forward-partition creator. P1 worker calls daily. If rows already landed in DEFAULT for the target month they are MOVED into the new partition (codex r5 P1-6 — no data loss). Holds ACCESS EXCLUSIVE on cost_findings briefly during the rare backfill case.';

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
