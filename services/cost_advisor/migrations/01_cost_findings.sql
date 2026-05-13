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

    -- Cheap pre-check: skip the lock acquisition if the partition is
    -- already there.
    IF to_regclass(v_part_name) IS NOT NULL THEN
        RETURN NULL;
    END IF;

    -- Codex r6 P1: concurrent callers can both pass the pre-check
    -- above; the loser waits on ACCESS EXCLUSIVE, then duplicate-table
    -- failure follows. Recheck AFTER acquiring the lock.
    LOCK TABLE cost_findings IN ACCESS EXCLUSIVE MODE;

    IF to_regclass(v_part_name) IS NOT NULL THEN
        -- Another concurrent caller won the race; nothing left to do.
        RETURN NULL;
    END IF;

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
-- Postgres requires UNIQUE constraints on partitioned tables to include
-- the partition key. Rules dedupe on `(tenant_id, fingerprint)` which
-- does not include `detected_at`, so we keep a non-partitioned mirror
-- table whose PK is the dedup key and points back at the canonical
-- finding row's `(detected_at, finding_id)` partition coordinates.
--
-- Codex r6 P1: writers MUST go through `cost_findings_upsert()` SP
-- below. Direct INSERT into either table without the SP can orphan a
-- mirror row or duplicate findings on retry boundaries.
CREATE TABLE cost_findings_fingerprint_keys (
    tenant_id   UUID NOT NULL,
    fingerprint CHAR(64) NOT NULL,
    finding_id  UUID NOT NULL,
    detected_at TIMESTAMPTZ NOT NULL,  -- partition pointer for the SP UPDATE path
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
    'Cost Advisor §4.1: derived findings emitted by rules. Idempotent UPSERT keyed by (tenant_id, fingerprint) via cost_findings_upsert() SP. Lifecycle: open → dismissed | fixed | superseded. Retention driven by tenant_data_policy.cost_findings_retention_days_* (ledger DB) + retention_sweeper (P1 sweep kind).';

-- =====================================================================
-- cost_findings_upsert: the SOLE legal writer entry point (codex r6 P1)
-- =====================================================================
--
-- Atomically inserts a new finding OR updates the existing one on
-- (tenant_id, fingerprint) collision. The function:
--   1. Tries to claim the (tenant_id, fingerprint) pair in the
--      non-partitioned mirror with INSERT ... ON CONFLICT DO NOTHING
--      RETURNING. Two outcomes:
--      a. Claim succeeded → INSERT into cost_findings with the new
--         finding_id; return ('inserted', finding_id).
--      b. Claim failed → existing finding wins; SELECT the partition
--         pointer (detected_at, finding_id) from the mirror, then
--         UPDATE cost_findings on that partition coordinate.
--         Return ('updated', existing_finding_id).
--
-- The whole thing runs in the caller's transaction. Concurrent
-- callers on the same fingerprint either see the same finding_id
-- (idempotent) or one INSERT wins + the rest become UPDATEs — no
-- duplicate findings, no orphan mirror rows.
--
-- Caller responsibilities:
--   * Compute fingerprint per spec §11.5 A1 BEFORE calling.
--   * Generate finding_id (UUID v7) BEFORE calling so the mirror has
--     a value even if no INSERT wins.
--   * Pass detected_at; the SP uses it as the partition key for
--     UPDATE-path lookups. For UPDATE path the caller's detected_at
--     is ignored; the mirror's stored detected_at is authoritative.

CREATE OR REPLACE FUNCTION cost_findings_upsert(
    p_finding_id          UUID,
    p_fingerprint         CHAR(64),
    p_tenant_id           UUID,
    p_detected_at         TIMESTAMPTZ,
    p_rule_id             TEXT,
    p_rule_version        INT,
    p_category            TEXT,
    p_severity            TEXT,
    p_confidence          NUMERIC,
    p_agent_id            TEXT,
    p_run_id              TEXT,
    p_contract_bundle_id  TEXT,
    p_evidence            JSONB,
    p_estimated_waste     BIGINT,
    p_sample_decision_ids UUID[]
) RETURNS TABLE (
    outcome           TEXT,                -- 'inserted' | 'updated' | 'reinstated'
    finding_id        UUID,
    finding_detected_at TIMESTAMPTZ
) LANGUAGE plpgsql AS $$
DECLARE
    v_claimed_finding_id UUID;
    v_existing_finding_id UUID;
    v_existing_detected_at TIMESTAMPTZ;
BEGIN
    -- Phase 1: try to claim the fingerprint slot.
    INSERT INTO cost_findings_fingerprint_keys
        (tenant_id, fingerprint, finding_id, detected_at)
        VALUES (p_tenant_id, p_fingerprint, p_finding_id, p_detected_at)
        ON CONFLICT (tenant_id, fingerprint) DO NOTHING
        RETURNING cost_findings_fingerprint_keys.finding_id
        INTO v_claimed_finding_id;

    IF v_claimed_finding_id IS NOT NULL THEN
        -- Phase 1a: claim succeeded → INSERT canonical row.
        INSERT INTO cost_findings (
            finding_id, fingerprint, tenant_id, detected_at,
            rule_id, rule_version, category, severity, confidence,
            agent_id, run_id, contract_bundle_id,
            evidence, estimated_waste_micros_usd, sample_decision_ids
        ) VALUES (
            p_finding_id, p_fingerprint, p_tenant_id, p_detected_at,
            p_rule_id, p_rule_version, p_category, p_severity, p_confidence,
            p_agent_id, p_run_id, p_contract_bundle_id,
            p_evidence, p_estimated_waste, p_sample_decision_ids
        );
        RETURN QUERY SELECT 'inserted'::TEXT, p_finding_id, p_detected_at;
        RETURN;
    END IF;

    -- Phase 1b: claim failed → look up existing pointer.
    SELECT m.finding_id, m.detected_at
      INTO v_existing_finding_id, v_existing_detected_at
      FROM cost_findings_fingerprint_keys m
     WHERE m.tenant_id = p_tenant_id AND m.fingerprint = p_fingerprint
     FOR UPDATE;

    -- Phase 2: refresh the canonical row at its known partition.
    UPDATE cost_findings SET
        evidence                   = p_evidence,
        severity                   = p_severity,
        confidence                 = p_confidence,
        estimated_waste_micros_usd = p_estimated_waste,
        sample_decision_ids        = p_sample_decision_ids
        -- created_at + finding_id + fingerprint + rule_id + rule_version
        -- + tenant_id + detected_at are NOT touched (the cost_findings_
        -- touch trigger updates updated_at).
     WHERE cost_findings.tenant_id   = p_tenant_id
       AND cost_findings.detected_at = v_existing_detected_at
       AND cost_findings.finding_id  = v_existing_finding_id;

    -- Codex r7 P1: stale-mirror hole. If retention / operator deleted
    -- the canonical row but the mirror survived, the UPDATE above
    -- silently updates zero rows. Detect via GET DIAGNOSTICS and
    -- self-heal: re-point the mirror at the caller's NEW
    -- (finding_id, detected_at) and re-INSERT the canonical row from
    -- the caller's data. The mirror's UNIQUE (tenant_id, fingerprint)
    -- holds across the re-point; the surviving guarantee is "every
    -- fingerprint has exactly one (mirror, canonical) pair".
    --
    -- Returns outcome='reinstated' so callers can distinguish a
    -- self-heal from a normal UPDATE for metrics / alerting.
    IF NOT FOUND THEN
        UPDATE cost_findings_fingerprint_keys
           SET finding_id = p_finding_id,
               detected_at = p_detected_at
         WHERE tenant_id = p_tenant_id AND fingerprint = p_fingerprint;

        INSERT INTO cost_findings (
            finding_id, fingerprint, tenant_id, detected_at,
            rule_id, rule_version, category, severity, confidence,
            agent_id, run_id, contract_bundle_id,
            evidence, estimated_waste_micros_usd, sample_decision_ids
        ) VALUES (
            p_finding_id, p_fingerprint, p_tenant_id, p_detected_at,
            p_rule_id, p_rule_version, p_category, p_severity, p_confidence,
            p_agent_id, p_run_id, p_contract_bundle_id,
            p_evidence, p_estimated_waste, p_sample_decision_ids
        );
        RETURN QUERY SELECT 'reinstated'::TEXT, p_finding_id, p_detected_at;
        RETURN;
    END IF;

    RETURN QUERY SELECT 'updated'::TEXT, v_existing_finding_id, v_existing_detected_at;
END;
$$;

COMMENT ON FUNCTION cost_findings_upsert IS
    'Cost Advisor §11.5 A1 + codex r6 P1: the SOLE legal writer entry point for cost_findings. Atomically claims (tenant_id, fingerprint) in the mirror then INSERTs or UPDATEs the canonical partition row. Direct INSERTs that skip this SP risk orphan mirror rows or duplicate findings on retry boundaries.';
