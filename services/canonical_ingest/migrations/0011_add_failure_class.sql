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

ALTER TABLE canonical_events
    ADD COLUMN failure_class TEXT NULL;

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
    ));

-- Partial index on rows where rules WILL fire. Most rows are
-- 'unknown' / NULL / unbilled — those are skipped by the rule SQL
-- (`WHERE failure_class IN (provable list)`), so an index covering
-- only the rule-relevant subset stays cheap.
CREATE INDEX canonical_events_failure_class_billed_idx
    ON canonical_events (tenant_id, recorded_month, failure_class)
    WHERE failure_class IN (
        'provider_5xx',
        'provider_4xx_billed',
        'malformed_json_response',
        'timeout_billed',
        'retry_then_success'
    );

COMMENT ON COLUMN canonical_events.failure_class IS
    'Cost Advisor §5.1.2: classifier-assigned failure class. NULL = not classified (pre-migration rows or non-LLM-call audits). Set at INSERT by canonical_ingest::classify (P1). Pre-existing rows stay NULL; canonical_events is append-only so they cannot be backfilled without an operational trigger-drop procedure.';
