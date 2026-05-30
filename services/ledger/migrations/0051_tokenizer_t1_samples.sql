-- ============================================================================
-- 0051_tokenizer_t1_samples.sql — Tier 1 shadow sample table.
--
-- Spec ancestors:
--   - tokenizer-service-spec-v1alpha1.md §4 (Tier 1 shadow drift detection)
--   - tokenizer-service-spec-v1alpha1.md §4.4 (this table's authoritative DDL)
--   - SLICE_05 §5 (this slice's schema owner)
--
-- ## Why this table is SEPARATE from audit_outbox
--
-- Per spec §4.4 Tier 1 shadow samples are **verification only** — they are NOT
-- enforcement source of truth for reservations. The audit chain holds the
-- Tier 2 (or Tier 3) result that the reservation was actually based on; the
-- Tier 1 sample is a parallel observation we use to detect vendor BPE drift.
--
-- Consequences:
--   * No immutability trigger — samples are mutable (operator can repair
--     a botched insert) and deletable (90-day retention cleanup).
--   * No signed CloudEvent emission per sample (those are too noisy at
--     ~100k samples/day per medium-tenant). What IS signed + audit-chain
--     bound is the `tokenizer_drift_alert` CloudEvent emitted when a
--     sample crosses the per-kind drift_threshold (per §4.2).
--   * NOT included in `verify-chain` audit replay — operators who need
--     to reproduce a historical decision should not consult this table.
--   * Retention: 90 days default. SLICE_05 ships the schema only; the
--     cleanup job (partition-by-month OR cron DELETE) is deferred to
--     SLICE-extra per the §11 risk plan. The index supports cheap
--     `DELETE WHERE sampled_at < now() - interval '90 days'` until then.
--
-- ## FK choice
--
-- `t2_tokenizer_version_id REFERENCES tokenizer_versions(tokenizer_version_id)`
-- with ON DELETE RESTRICT — same posture as audit_outbox.tokenizer_version_id
-- (0048:154-161). Losing the encoder identity for a historical sample would
-- defeat the drift-attribution workflow (operator needs to know "which BPE
-- version drifted vs which provider count").
--
-- ## Privilege boundary
--
-- Mirrors 0048 / 0049: REVOKE PUBLIC + GRANT to ledger_application_role for
-- INSERT (shadow worker writes), GRANT SELECT to ledger_reader_role (drift
-- analysis CLI reads). DELETE is granted to the application role for the
-- 90-day cleanup job; UPDATE remains revoked since samples are
-- write-once after the drift_ratio is computed.
--
-- ## Stylistic alignment (Codex m3 convention, per 0050)
--
-- Migration runner uses psql autocommit per SLICE_01 R5; each statement
-- commits independently. No explicit BEGIN/COMMIT.
-- DO blocks use `SET LOCAL search_path = pg_catalog, pg_temp` (SLICE_01 R5
-- convention; CVE-2018-1058 hardening).
--
-- ## Down migration
--
-- Per SLICE_03 R2 M3 convention there is no separate 0051_down file; the
-- DROP TABLE here is documented in this header so a rollback is a manual
-- one-liner (`DROP TABLE tokenizer_t1_samples CASCADE`) for the operator.
-- ============================================================================

-- ============================================================================
-- Partitioning (R2 M8): RANGE-partitioned by sampled_at, monthly partitions.
-- Same pattern as 0009_audit_outbox.sql — drops become `DROP TABLE
-- tokenizer_t1_samples_YYYYMM` after the 90-day retention window, far
-- cheaper than `DELETE WHERE sampled_at < cutoff` on a ~10M-row heap.
--
-- Partition key MUST be part of every UNIQUE constraint (Postgres §5.11);
-- PRIMARY KEY therefore becomes `(sample_id, sampled_at)`.
-- ============================================================================

CREATE TABLE tokenizer_t1_samples (
    -- Application-minted UUIDv7; no DEFAULT so the writer (shadow worker)
    -- is forced to mint it explicitly — mirrors 0048 tokenizer_versions
    -- and 0049 seed conventions.
    sample_id              UUID         NOT NULL,

    -- Tenant + model are the per-(tenant, model) state key for sample rate,
    -- cool-down, and circuit breaker. tenant_id is UUID matching the rest
    -- of the ledger schema (0003 budget_window_instances, 0028
    -- tenant_data_policy, 0034 invoice_reconcile_decision_producer_id, etc.).
    -- model is vendor-defined opaque identifier (e.g. "claude-3-5-sonnet").
    tenant_id              UUID         NOT NULL,
    model                  TEXT         NOT NULL,

    -- Wallclock at sample observation. TIMESTAMPTZ with TZ-explicit storage
    -- per SLICE_01 R5 convention. R2 M8: event-time semantics — the
    -- shadow worker passes the observation timestamp via explicit INSERT
    -- so a slow worker queue does not skew the retention math.
    sampled_at             TIMESTAMPTZ  NOT NULL DEFAULT clock_timestamp(),

    -- Token counts: Tier 1 (provider count_tokens API) and Tier 2 (vendored
    -- BPE) for the same input. INT covers the spec §10.1 1 MiB raw_text /
    -- 256k token upper bound with room to spare.
    t1_input_tokens        INT          NOT NULL CHECK (t1_input_tokens >= 0),
    t2_input_tokens        INT          NOT NULL CHECK (t2_input_tokens >= 0),

    -- FK to the encoder that produced t2_input_tokens. Critical for drift
    -- attribution: operator needs to know "which BPE version drifted".
    t2_tokenizer_version_id UUID        NOT NULL
                            REFERENCES public.tokenizer_versions(tokenizer_version_id)
                            ON DELETE RESTRICT,

    -- |t1 - t2| / max(t1, 1) — REAL is sufficient precision (we alert on
    -- 1% thresholds, not 1e-6 deltas). NOT NULL because the sample insert
    -- always knows the value.
    drift_ratio            REAL         NOT NULL CHECK (drift_ratio >= 0.0),

    -- R2 M9: SEMANTICS — this column reflects the *decision* the worker
    -- made (drift_ratio > threshold). The CloudEvent emission outcome is
    -- tracked separately by drift_alert_emitted_at. Renamed from
    -- drift_alert_emitted (which conflated decision with emission ack).
    drift_alert_decided    BOOLEAN      NOT NULL,

    -- R2 M9: timestamp at which the emit_drift_alert path successfully
    -- forwarded the CloudEvent to canonical_ingest. NULL when:
    --   * drift_alert_decided = FALSE (no emission attempted), or
    --   * emission failed (worker retries log + skip; row is still
    --     persisted so the sample is not lost).
    -- Indexed below for "show me alerts that actually emitted".
    drift_alert_emitted_at TIMESTAMPTZ  NULL,

    -- Optional provider-side request id (Anthropic returns one in
    -- x-request-id; Gemini in `name`). NULL when the provider call
    -- failed before a response id was returned.
    provider_request_id    TEXT,

    -- R2 M8 partition constraint — partition key (sampled_at) must be
    -- part of every UNIQUE constraint on the parent table.
    PRIMARY KEY (sample_id, sampled_at)
) PARTITION BY RANGE (sampled_at);

-- R3 N2 fix: pre-create current + 2 future monthly partitions (2026-05
-- ship window: 2026-05, 2026-06, 2026-07). No DEFAULT partition — a
-- missing-month INSERT must raise `no partition of relation` rather
-- than silently routing into a catch-all that defeats the
-- `DROP TABLE tokenizer_t1_samples_YYYYMM` retention model.
--
-- ## Operator obligation
--
-- Future-month partitions are NOT auto-created. Operators MUST add
-- the next month's partition before the 1st of that month, otherwise
-- shadow inserts after `2026-08-01 00:00:00+00` will fail with a
-- partition-routing error (and the worker fails the sample insert,
-- which is preferable to silent data loss in a default partition).
--
-- A cron job to mint future partitions (and DROP TABLE old ones for
-- the 90-day retention window) is deferred to SLICE-extra and tracked
-- as a GH issue (R3 N2 follow-up). The shadow worker is fail-loud on
-- a missing partition: the sample insert returns an error, the worker
-- logs + skips (sample is dropped, not silently corrupted), and the
-- `tokenizer_shadow_sample_insert_failed_total` Prometheus counter
-- ticks — operators see the alert and ship a partition.
--
-- ## Migration runner note
--
-- Each CREATE TABLE commits independently (psql autocommit per SLICE_01
-- R5). A partition-creation failure aborts the migration at the failing
-- statement; preceding partitions remain.
CREATE TABLE tokenizer_t1_samples_2026_05 PARTITION OF tokenizer_t1_samples
    FOR VALUES FROM ('2026-05-01 00:00:00+00') TO ('2026-06-01 00:00:00+00');

CREATE TABLE tokenizer_t1_samples_2026_06 PARTITION OF tokenizer_t1_samples
    FOR VALUES FROM ('2026-06-01 00:00:00+00') TO ('2026-07-01 00:00:00+00');

CREATE TABLE tokenizer_t1_samples_2026_07 PARTITION OF tokenizer_t1_samples
    FOR VALUES FROM ('2026-07-01 00:00:00+00') TO ('2026-08-01 00:00:00+00');

-- Per spec §4.4 — partial index for the alerting subset that successfully
-- emitted a CloudEvent (R2 M9: the operator-visible alert surface is the
-- set whose CloudEvent landed in canonical_ingest, not the set the
-- worker decided to alert on). Drives the most common operator query
-- "show me the alerts for tenant X in the last 24h" without scanning
-- the full retention window of normal samples (which can exceed 1M
-- rows for medium tenants at 1% sampling × normal traffic).
CREATE INDEX tokenizer_t1_samples_alert_idx
    ON tokenizer_t1_samples (tenant_id, model, sampled_at DESC)
    WHERE drift_alert_emitted_at IS NOT NULL;

-- Retention cleanup index — supports `DELETE WHERE sampled_at < cutoff`
-- without a sequential scan. Cheap because sampled_at advances
-- monotonically; index size is bounded by 90-day retention × insert rate.
CREATE INDEX tokenizer_t1_samples_retention_idx
    ON tokenizer_t1_samples (sampled_at);

-- ============================================================================
-- Privilege boundary (mirror of 0048 / 0049 lock-down).
-- ============================================================================

REVOKE INSERT, UPDATE, DELETE ON tokenizer_t1_samples FROM PUBLIC;

-- Shadow worker (running under ledger_application_role) inserts samples
-- and the cleanup job deletes them after 90 days.
GRANT INSERT, DELETE ON tokenizer_t1_samples TO ledger_application_role;

-- R2 M9: column-level UPDATE for drift_alert_emitted_at only. The shadow
-- worker writes this column AFTER successful canonical_ingest emission;
-- every other column remains write-once. Operators inspecting a row find
-- (decided=TRUE, emitted_at=NULL) when emission failed — useful for
-- diagnosing canonical_ingest outages without losing the drift sample.
GRANT UPDATE (drift_alert_emitted_at) ON tokenizer_t1_samples TO ledger_application_role;

-- Reader role for drift-analysis CLI + dashboards.
GRANT SELECT ON tokenizer_t1_samples TO ledger_reader_role;

COMMENT ON TABLE tokenizer_t1_samples IS
    'Tier 1 (provider count_tokens) shadow samples per tokenizer-service-spec-v1alpha1.md §4.4. Verification-only — NOT in audit chain. Retention: 90 days (cleanup job in SLICE-extra). Drift alerts cross into audit chain via signed tokenizer_drift_alert CloudEvents (not per-sample). R2 M8: PARTITION BY RANGE(sampled_at) with monthly partitions; DROP TABLE tokenizer_t1_samples_YYYYMM replaces the legacy DELETE retention path.';

-- ============================================================================
-- R2 M7: DO-block smoke check removed. Migration runner uses psql
-- autocommit per SLICE_01 R5 — each CREATE INDEX statement commits
-- before the next runs, so a CREATE INDEX failure aborts the migration
-- file at the failing statement rather than being masked by a wrapping
-- transaction. The downstream subsequent statements (REVOKE / GRANT /
-- COMMENT ON) implicitly prove the table + columns exist via their
-- references; a missing index surfaces at the first query that needs
-- it, with the same error as a "did we forget to migrate" check.
-- ============================================================================
