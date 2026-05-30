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
-- No explicit BEGIN / COMMIT — migration runner wraps the file in its own tx.
-- DO blocks use `SET LOCAL search_path = pg_catalog, pg_temp` (SLICE_01 R5
-- convention; CVE-2018-1058 hardening).
--
-- ## Down migration
--
-- Per SLICE_03 R2 M3 convention there is no separate 0051_down file; the
-- DROP TABLE here is documented in this header so a rollback is a manual
-- one-liner (`DROP TABLE tokenizer_t1_samples CASCADE`) for the operator.
-- ============================================================================

CREATE TABLE tokenizer_t1_samples (
    -- Application-minted UUIDv7; no DEFAULT so the writer (shadow worker)
    -- is forced to mint it explicitly — mirrors 0048 tokenizer_versions
    -- and 0049 seed conventions.
    sample_id              UUID         PRIMARY KEY,

    -- Tenant + model are the per-(tenant, model) state key for sample rate,
    -- cool-down, and circuit breaker. TEXT for both because tenant ids in
    -- some deployments are non-UUID (per SLICE_01 R7 audit convention) and
    -- model strings are vendor-defined opaque identifiers.
    tenant_id              TEXT         NOT NULL,
    model                  TEXT         NOT NULL,

    -- Wallclock at sample observation. TIMESTAMPTZ with TZ-explicit storage
    -- per SLICE_01 R5 convention.
    sampled_at             TIMESTAMPTZ  NOT NULL DEFAULT clock_timestamp(),

    -- Token counts: Tier 1 (provider count_tokens API) and Tier 2 (vendored
    -- BPE) for the same input. INT covers the spec §10.1 1 MiB raw_text /
    -- 256k token upper bound with room to spare.
    t1_input_tokens        INT          NOT NULL CHECK (t1_input_tokens >= 0),
    t2_input_tokens        INT          NOT NULL CHECK (t2_input_tokens >= 0),

    -- FK to the encoder that produced t2_input_tokens. Critical for drift
    -- attribution: operator needs to know "which BPE version drifted".
    t2_tokenizer_version_id UUID        NOT NULL
                            REFERENCES tokenizer_versions(tokenizer_version_id)
                            ON DELETE RESTRICT,

    -- |t1 - t2| / max(t1, 1) — REAL is sufficient precision (we alert on
    -- 1% thresholds, not 1e-6 deltas). NOT NULL because the sample insert
    -- always knows the value.
    drift_ratio            REAL         NOT NULL CHECK (drift_ratio >= 0.0),

    -- True iff this sample crossed the per-kind drift_threshold from
    -- §4.2 and the shadow worker emitted a `tokenizer_drift_alert`
    -- CloudEvent. Indexed below for "show me the alerting samples"
    -- queries common to incident response.
    drift_alert_emitted    BOOLEAN      NOT NULL DEFAULT FALSE,

    -- Optional provider-side request id (Anthropic returns one in
    -- x-request-id; Gemini in `name`). NULL when the provider call
    -- failed before a response id was returned.
    provider_request_id    TEXT
);

-- Per spec §4.4 — partial index for the alerting subset. Drives the most
-- common operator query "show me the alerts for tenant X in the last 24h"
-- without scanning the full retention window of normal samples (which can
-- exceed 1M rows for medium tenants at 1% sampling × normal traffic).
CREATE INDEX tokenizer_t1_samples_alert_idx
    ON tokenizer_t1_samples (tenant_id, model, sampled_at DESC)
    WHERE drift_alert_emitted = TRUE;

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

-- UPDATE deliberately NOT granted: samples are write-once after the
-- drift_ratio is computed. An update would mask a real drift event.

-- Reader role for drift-analysis CLI + dashboards.
GRANT SELECT ON tokenizer_t1_samples TO ledger_reader_role;

COMMENT ON TABLE tokenizer_t1_samples IS
    'Tier 1 (provider count_tokens) shadow samples per tokenizer-service-spec-v1alpha1.md §4.4. Verification-only — NOT in audit chain. Retention: 90 days (cleanup job in SLICE-extra). Drift alerts cross into audit chain via signed tokenizer_drift_alert CloudEvents (not per-sample).';

-- ============================================================================
-- Smoke check: confirm the indexes exist after CREATE TABLE so a partial
-- failure surfaces during the migration, not at the first cool-down query.
-- ============================================================================

DO $$
DECLARE
    expected_indexes INTEGER := 2;
    actual_indexes   INTEGER;
BEGIN
    SET LOCAL search_path = pg_catalog, pg_temp;
    SELECT COUNT(*) INTO actual_indexes
    FROM pg_indexes
    WHERE tablename = 'tokenizer_t1_samples'
      AND indexname IN ('tokenizer_t1_samples_alert_idx',
                        'tokenizer_t1_samples_retention_idx');
    IF actual_indexes <> expected_indexes THEN
        RAISE EXCEPTION
            'tokenizer_t1_samples migration sanity check failed: expected % indexes, got %',
            expected_indexes, actual_indexes;
    END IF;
END $$;
