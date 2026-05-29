-- Round-3 fix B2: extend audit_outcome_quarantine with the 18 prediction
-- columns so the release-from-quarantine path can re-hydrate them when
-- promoting a quarantined outcome to canonical_events.
--
-- Spec ancestor: docs/audit-chain-prediction-extension-v1alpha1.md §11.2
-- (cross-storage consistency) + SLICE_01 §6 (mirror invariant).
--
-- ## Why this matters
--
-- Without these columns the quarantine table is the only path where an
-- audit.outcome can land on canonical_events. The 0013 migration added
-- the 18 columns on canonical_events directly but did NOT mirror them
-- onto audit_outcome_quarantine. As a result:
--
--   1. Producer writes an audit.outcome with tag-300+ fields populated.
--   2. canonical_ingest decodes it, sees no preceding audit.decision,
--      routes to quarantine via insert into audit_outcome_quarantine.
--   3. Quarantine row stores cloudevent_payload (which carries the
--      prediction tags as proto bytes) but has nowhere to store the
--      decoded first-class column values.
--   4. The matching .decision arrives later; release_quarantined_outcomes()
--      reads the quarantine row + INSERTs into canonical_events.
--   5. Because the quarantine row has no first-class prediction columns,
--      the INSERT into canonical_events lands them all as NULL — even
--      though the proto bytes inside cloudevent_payload contain the
--      values.
--   6. The CHECK constraint
--      canonical_events_outcome_required_cols_chk (event_time >= 2026-07-01)
--      fires because actual_input_tokens / actual_output_tokens are NULL.
--      The release transaction aborts. The quarantine row gets stuck
--      forever in 'awaiting_decision' state.
--   7. Bonus failure: even if the CHECK passed, the verify-chain mirror
--      cross-check would flag a divergence (column NULL vs proto-encoded
--      value).
--
-- Fix: mirror the same 18 nullable columns onto the quarantine table so
-- the release path can carry them forward.
--
-- ## No backfill, no CHECK constraints on quarantine
--
-- The quarantine table is short-lived staging — rows are released or
-- orphaned within 30s per spec §4.8. We DO NOT mirror the CHECK
-- constraints from 0013 here because:
--   * The producer-side has already validated them BEFORE writing the
--     CloudEvent payload (see crates/spendguard-prediction-mirror).
--   * The canonical_events INSERT at release time will re-validate.
--   * Duplicating CHECKs would force a second VALIDATE pass.
--
-- ## No backfill
--
-- Existing quarantine rows (pre-SLICE_01) have NULL on the new columns.
-- Those rows are either already released (state='released'; new columns
-- stay NULL, signature still verifies because the CloudEvent payload is
-- already on canonical_events) or orphaned (state='orphaned'; never
-- reaches canonical_events). No retroactive fixup needed.

ALTER TABLE audit_outcome_quarantine
    -- === Decision-side prediction columns (11 per spec §2.1) ===
    ADD COLUMN predicted_a_tokens         BIGINT,
    ADD COLUMN predicted_b_tokens         BIGINT,
    ADD COLUMN predicted_c_tokens         BIGINT,
    ADD COLUMN reserved_strategy          TEXT,
    ADD COLUMN prediction_strategy_used   TEXT,
    ADD COLUMN prediction_policy_used     TEXT,
    ADD COLUMN tokenizer_tier             TEXT,
    ADD COLUMN tokenizer_version_id       UUID,
    ADD COLUMN prediction_confidence      NUMERIC(4,3),
    ADD COLUMN prediction_sample_size     BIGINT,
    ADD COLUMN cold_start_layer_used      TEXT,
    -- === Run-level projection columns (3 per spec §2.2) ===
    ADD COLUMN run_projection_at_decision_atomic NUMERIC(38,0),
    ADD COLUMN run_predicted_remaining_steps     INT,
    ADD COLUMN run_steps_completed_so_far        BIGINT,
    -- === Commit-side actual columns (4 per spec §2.3) ===
    ADD COLUMN actual_input_tokens   BIGINT,
    ADD COLUMN actual_output_tokens  BIGINT,
    ADD COLUMN delta_b_ratio         REAL,
    ADD COLUMN delta_c_ratio         REAL;

-- ============================================================================
-- Round-3 fix B2: extend the quarantine immutability trigger to lock
-- the 18 new columns. Otherwise a tampering UPDATE could rewrite a
-- prediction column inside the quarantine while state still
-- 'awaiting_decision', then the release path would carry the tampered
-- value into canonical_events — bypassing both the canonical_events
-- immutability trigger AND the verify-chain mirror cross-check (since
-- the producer's CloudEvent signature covers the proto payload, not the
-- mirrored column).
-- ============================================================================

CREATE OR REPLACE FUNCTION reject_quarantine_immutable_columns()
RETURNS TRIGGER AS $$
BEGIN
    IF (OLD.quarantine_id, OLD.event_id, OLD.tenant_id, OLD.decision_id,
        OLD.storage_class, OLD.producer_id, OLD.producer_sequence,
        OLD.producer_signature, OLD.signing_key_id, OLD.schema_bundle_id,
        OLD.schema_bundle_hash, OLD.event_type, OLD.specversion, OLD.source,
        OLD.event_time, OLD.datacontenttype, OLD.payload_json,
        OLD.payload_blob_ref, OLD.region_id, OLD.ingest_shard_id,
        OLD.ingest_log_offset, OLD.run_id, OLD.quarantined_at,
        OLD.orphan_after,
        -- === NEW prediction columns (round-3 B2) ===
        OLD.predicted_a_tokens, OLD.predicted_b_tokens, OLD.predicted_c_tokens,
        OLD.reserved_strategy, OLD.prediction_strategy_used,
        OLD.prediction_policy_used, OLD.tokenizer_tier, OLD.tokenizer_version_id,
        OLD.prediction_confidence, OLD.prediction_sample_size,
        OLD.cold_start_layer_used,
        OLD.run_projection_at_decision_atomic,
        OLD.run_predicted_remaining_steps,
        OLD.run_steps_completed_so_far,
        OLD.actual_input_tokens, OLD.actual_output_tokens,
        OLD.delta_b_ratio, OLD.delta_c_ratio)
       IS DISTINCT FROM
       (NEW.quarantine_id, NEW.event_id, NEW.tenant_id, NEW.decision_id,
        NEW.storage_class, NEW.producer_id, NEW.producer_sequence,
        NEW.producer_signature, NEW.signing_key_id, NEW.schema_bundle_id,
        NEW.schema_bundle_hash, NEW.event_type, NEW.specversion, NEW.source,
        NEW.event_time, NEW.datacontenttype, NEW.payload_json,
        NEW.payload_blob_ref, NEW.region_id, NEW.ingest_shard_id,
        NEW.ingest_log_offset, NEW.run_id, NEW.quarantined_at,
        NEW.orphan_after,
        NEW.predicted_a_tokens, NEW.predicted_b_tokens, NEW.predicted_c_tokens,
        NEW.reserved_strategy, NEW.prediction_strategy_used,
        NEW.prediction_policy_used, NEW.tokenizer_tier, NEW.tokenizer_version_id,
        NEW.prediction_confidence, NEW.prediction_sample_size,
        NEW.cold_start_layer_used,
        NEW.run_projection_at_decision_atomic,
        NEW.run_predicted_remaining_steps,
        NEW.run_steps_completed_so_far,
        NEW.actual_input_tokens, NEW.actual_output_tokens,
        NEW.delta_b_ratio, NEW.delta_c_ratio) THEN
        RAISE EXCEPTION 'audit_outcome_quarantine immutable columns cannot be changed (incl. prediction extension cols)'
            USING ERRCODE = '42P10';
    END IF;
    -- Allowed transitions: awaiting_decision -> released | orphaned.
    IF OLD.state = 'released' OR OLD.state = 'orphaned' THEN
        RAISE EXCEPTION 'audit_outcome_quarantine state is terminal'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

COMMENT ON COLUMN audit_outcome_quarantine.predicted_a_tokens IS
    'Round-3 B2: carries the audit.outcome''s tag-300+ prediction context through quarantine staging so release_quarantined_outcomes() can populate canonical_events with first-class columns matching the embedded CloudEvent payload.';
