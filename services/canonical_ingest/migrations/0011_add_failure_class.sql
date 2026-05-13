-- Cost Advisor P0: failure_class column on canonical_events.
--
-- Spec: docs/specs/cost-advisor-spec.md §5.1.2 (failure classification
-- ownership).
--
-- Adds a typed `failure_class` column whose values are owned by the
-- `canonical_ingest` service's new `classify.rs` module (lands in P1).
-- Rules in `services/cost_advisor/` read this column instead of doing
-- classification at query time, so the hard logic (HTTP status × usage
-- field × framework signature) is centralized, tested, and versioned.
--
-- Backfill semantics:
--   * Pre-migration rows have `failure_class = NULL` permanently.
--     Rules treat NULL as "not classified" (= "do not fire waste
--     verdicts on this row"). This is the safe degraded behavior.
--   * `canonical_events` is append-only (see 0005_immutability_triggers).
--     Backfill via UPDATE is BLOCKED by `canonical_events_no_update_delete`.
--     Operational backfill (if business requires re-classifying
--     historical data) must follow a separate procedure that
--     temporarily drops the trigger; not in scope for this migration.
--   * Forward population happens at INSERT time in canonical_ingest's
--     AppendEvents handler (P1 wiring).
--
-- CLASSIFIER_VERSION (the classify.rs constant per §5.1.2) is tracked
-- as a regular code artifact, not in a column. Re-classification of
-- events <= 30 days old after a CLASSIFIER_VERSION bump is the same
-- "temporary trigger drop" operational procedure as a backfill.

-- Step 1: column add. Postgres 16: nullable column without default is
-- metadata-only — brief ACCESS EXCLUSIVE for catalog flip, no row
-- rewrite. Safe on a hot append-only audit table.
ALTER TABLE canonical_events
    ADD COLUMN failure_class TEXT NULL;

-- Step 2: CHECK constraint with NOT VALID — metadata-only, no scan.
-- A subsequent VALIDATE CONSTRAINT runs an online scan under SHARE
-- UPDATE EXCLUSIVE (concurrent INSERTs proceed). Without NOT VALID
-- the ADD CONSTRAINT would scan under ACCESS EXCLUSIVE — blocking
-- canonical_ingest on the hot audit table for the scan duration
-- (codex r5 P1-3).
--
-- Enum allowlist per spec §5.1.2. CHECK admits NULL so rows that
-- aren't LLM-call audits (e.g. denied decisions, releases) and
-- pre-migration rows remain valid.
ALTER TABLE canonical_events
    ADD CONSTRAINT canonical_events_failure_class_enum
    CHECK (failure_class IS NULL OR failure_class IN (
        'unknown',
        'provider_5xx',
        'provider_4xx_billed',
        'provider_4xx_unbilled',
        'tool_error',
        'malformed_json_response',
        'timeout_billed',
        'timeout_unbilled',
        'retry_then_success'
    ))
    NOT VALID;

ALTER TABLE canonical_events
    VALIDATE CONSTRAINT canonical_events_failure_class_enum;

-- Step 3: partial index. canonical_events is RANGE-partitioned by
-- recorded_month. CREATE INDEX directly on the parent locks every
-- partition AccessExclusive for the build duration — codex r5 P1-4.
--
-- Postgres-recommended online-safe pattern (PG16 docs §5.11.2.2):
--   a. Create the parent index ON ONLY — invalid placeholder, no scan.
--   b. CREATE INDEX CONCURRENTLY per partition — online build.
--   c. ALTER INDEX <parent> ATTACH PARTITION <child> for each child;
--      parent index becomes VALID once all children attached.
--
-- CREATE INDEX CONCURRENTLY cannot run inside an explicit BEGIN/COMMIT
-- block; psql's `-f` runs each statement in its own implicit tx so
-- this file is safe.

CREATE INDEX canonical_events_failure_class_billed_idx
    ON ONLY canonical_events (tenant_id, recorded_month, failure_class)
    WHERE failure_class IN (
        'provider_5xx',
        'provider_4xx_billed',
        'malformed_json_response',
        'timeout_billed',
        'retry_then_success'
    );

CREATE INDEX CONCURRENTLY canonical_events_2026_05_failure_class_idx
    ON canonical_events_2026_05 (tenant_id, recorded_month, failure_class)
    WHERE failure_class IN (
        'provider_5xx',
        'provider_4xx_billed',
        'malformed_json_response',
        'timeout_billed',
        'retry_then_success'
    );

CREATE INDEX CONCURRENTLY canonical_events_2026_06_failure_class_idx
    ON canonical_events_2026_06 (tenant_id, recorded_month, failure_class)
    WHERE failure_class IN (
        'provider_5xx',
        'provider_4xx_billed',
        'malformed_json_response',
        'timeout_billed',
        'retry_then_success'
    );

CREATE INDEX CONCURRENTLY canonical_events_2026_07_failure_class_idx
    ON canonical_events_2026_07 (tenant_id, recorded_month, failure_class)
    WHERE failure_class IN (
        'provider_5xx',
        'provider_4xx_billed',
        'malformed_json_response',
        'timeout_billed',
        'retry_then_success'
    );

CREATE INDEX CONCURRENTLY canonical_events_default_failure_class_idx
    ON canonical_events_default (tenant_id, recorded_month, failure_class)
    WHERE failure_class IN (
        'provider_5xx',
        'provider_4xx_billed',
        'malformed_json_response',
        'timeout_billed',
        'retry_then_success'
    );

ALTER INDEX canonical_events_failure_class_billed_idx
    ATTACH PARTITION canonical_events_2026_05_failure_class_idx;
ALTER INDEX canonical_events_failure_class_billed_idx
    ATTACH PARTITION canonical_events_2026_06_failure_class_idx;
ALTER INDEX canonical_events_failure_class_billed_idx
    ATTACH PARTITION canonical_events_2026_07_failure_class_idx;
ALTER INDEX canonical_events_failure_class_billed_idx
    ATTACH PARTITION canonical_events_default_failure_class_idx;

COMMENT ON COLUMN canonical_events.failure_class IS
    'Cost Advisor §5.1.2: classifier-assigned failure class. NULL = not classified (pre-migration rows or non-LLM-call audits). Set at INSERT by canonical_ingest::classify (P1). Pre-existing rows stay NULL; canonical_events is append-only so they cannot be backfilled without an operational trigger-drop procedure.';
