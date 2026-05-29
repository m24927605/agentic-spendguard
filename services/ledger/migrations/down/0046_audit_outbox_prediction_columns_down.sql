-- Down-migration: reverse 0046_audit_outbox_prediction_columns.sql
-- (round-2 fix m2; round-3 fixes M4 + M10).
--
-- Spec ref: docs/slices/SLICE_01_canonical_events_migration.md §11
--   rollback plan.
--
-- ## Rollback order (per slice §11; round-4 fix B1 — header rewritten to
-- match slice doc which is authoritative)
--
--   1. Stop all 4 producer services so no new tag-300+ writes happen.
--   2. Apply canonical_ingest down-migrations on the canonical DB:
--      0015_down → 0013_down. 0015_down first so quarantine columns are
--      dropped before the canonical_events columns they mirror.
--   3. Apply ledger down-migrations on the ledger DB in this order:
--      0048_down → THIS FILE (0046_down). 0048_down drops the FK on
--      audit_outbox.tokenizer_version_id BEFORE this file drops the
--      column itself — reversing the order would fail because the FK
--      points at a column that no longer exists.
--   4. After this file finishes, the schema substrate matches the
--      pre-SLICE_01 state.
--   5. Roll back canonical_ingest pods to the pre-SLICE_01 image.
--   6. Roll back producer pods to the pre-SLICE_01 image.
--
-- ## Idempotency
--
-- All ALTER TABLE ... DROP COLUMN are guarded with IF EXISTS so re-running
-- after partial rollback is safe. DROP TRIGGER IF EXISTS likewise.
--
-- ## Round-3 fix M4 + round-4 fix M4: destructive-down guard
--
-- This file drops 18 columns + 3 partitions + 2 triggers + 1 function.
-- To prevent accidental application against production we gate the entire
-- file on a session-local GUC scoped per migration file. Operator must
-- explicitly opt in via:
--     SET LOCAL spendguard.allow_destructive_down_0046 = on;
-- BEFORE running the file. Without the GUC the first statement raises an
-- exception and the migration runner aborts.
--
-- Round-4 M4 rename: the GUC name now embeds the migration number so a
-- single SET LOCAL ... = on cannot cascade across multiple destructive
-- down files. Each file requires its own SET. The exact form to use is
-- documented in docs/slices/SLICE_01_canonical_events_migration.md §11
-- rollback runbook with the quoting convention.
--
-- Also emits a RAISE NOTICE 'DESTRUCTIVE down-migration 0046 proceeding
-- (caller: <role>)' so PG audit / log_min_messages = notice captures
-- the rollback for forensics.
--
-- ## Round-3 fix M4 + round-4 fix B6 + round-5 fix N12-B: partition row-count guards
--
-- DROP TABLE on a partition with rows is silently destructive. Before each
-- partition drop we LOCK TABLE ... IN ACCESS EXCLUSIVE MODE then
-- check row count and refuse if non-zero. Operator must manually
-- migrate (or accept loss) before running again.
--
-- Round-4 B6: the LOCK upgrade closed the TOCTOU window where a concurrent
-- INSERT between COUNT(*) and DROP TABLE could see a zero count and have
-- its row silently destroyed.
--
-- Round-5 fix N12-B (Option B systemic fix): the migration runner invokes
-- psql WITHOUT --single-transaction, so every top-level statement
-- autocommits. Under round-4's pattern — DO $$ ... LOCK + COUNT END $$ on
-- one line, DROP TABLE on the next — the lock is released the moment the
-- DO block's implicit transaction commits, then the bare DROP TABLE
-- starts a fresh transaction without re-acquiring it. A concurrent INSERT
-- between commit-of-DO and start-of-DROP would silently lose its row.
-- Round-5 collapses LOCK + COUNT + DROP into a single DO $$ block using
-- EXECUTE 'DROP TABLE ...' so the three operations share one implicit
-- transaction regardless of runner mode. ACCESS EXCLUSIVE held until END
-- $$ commits the whole block, blocking all reads + writes for the entire
-- count → drop sequence.
--
-- ## Round-3 fix M10: no BEGIN/COMMIT
--
-- Migration runner wraps each .sql in its own transaction; explicit
-- BEGIN/COMMIT was a round-2 artifact that diverged from the up-migration
-- convention. Removed.
--
-- ## Data loss warning
--
-- Dropping the 18 columns is destructive. Any row written by SLICE_06+
-- producers will lose its prediction columns; the CloudEvent proto
-- payload in cloudevent_payload still carries the tag-300+ data, but
-- the SQL-side accelerators are gone. calibration-report stops working
-- until the columns are re-added.
--
-- For non-destructive rollback (e.g., temporary disable while debugging
-- a producer bug), prefer reverting only the producer image without
-- touching the schema — the columns sit unused but harmless.

-- ============================================================================
-- Destructive-down guard (round-3 fix M4; round-4 fix M4 per-file scope +
-- caller audit notice).
-- ============================================================================
DO $$
BEGIN
    IF current_setting('spendguard.allow_destructive_down_0046', true) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'destructive down-migration 0046 requires `SET LOCAL spendguard.allow_destructive_down_0046 = on` first';
    END IF;
    RAISE NOTICE 'DESTRUCTIVE down-migration 0046 proceeding (caller: %)', current_user;
END $$;

-- Restore the pre-SLICE_01 trigger function (from
-- services/ledger/migrations/0011_immutability_triggers.sql).
CREATE OR REPLACE FUNCTION reject_audit_outbox_immutable_columns()
RETURNS TRIGGER AS $$
BEGIN
    IF (OLD.audit_outbox_id, OLD.audit_decision_event_id, OLD.decision_id,
        OLD.tenant_id, OLD.ledger_transaction_id, OLD.event_type,
        OLD.cloudevent_payload, OLD.cloudevent_payload_signature,
        OLD.ledger_fencing_epoch, OLD.workload_instance_id,
        OLD.recorded_at, OLD.recorded_month,
        OLD.producer_sequence, OLD.idempotency_key)
       IS DISTINCT FROM
       (NEW.audit_outbox_id, NEW.audit_decision_event_id, NEW.decision_id,
        NEW.tenant_id, NEW.ledger_transaction_id, NEW.event_type,
        NEW.cloudevent_payload, NEW.cloudevent_payload_signature,
        NEW.ledger_fencing_epoch, NEW.workload_instance_id,
        NEW.recorded_at, NEW.recorded_month,
        NEW.producer_sequence, NEW.idempotency_key) THEN
        RAISE EXCEPTION 'audit_outbox immutable columns cannot be changed'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Drop the TRUNCATE guard added in 0046 step 6.
DROP TRIGGER IF EXISTS audit_outbox_no_truncate ON audit_outbox;

-- Drop the generic TRUNCATE-rejector function (round-3 M6). Note: 0048's
-- tokenizer_versions_no_truncate trigger reuses this function. We drop
-- the trigger here and let 0048's own down-migration handle its TRUNCATE
-- trigger separately. The function itself is dropped here because no
-- other table uses it after SLICE_01 rollback.
DROP FUNCTION IF EXISTS reject_truncate_on_immutable_table();

-- Drop CHECK constraints. Order doesn't matter; they are independent.
ALTER TABLE audit_outbox
    DROP CONSTRAINT IF EXISTS audit_outbox_reserved_strategy_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_prediction_strategy_used_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_prediction_policy_used_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_tokenizer_tier_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_prediction_confidence_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_cold_start_layer_used_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_predicted_tokens_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_actual_tokens_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_run_steps_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_run_projection_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_run_projection_int64_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_prediction_sample_size_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_delta_b_ratio_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_delta_c_ratio_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_decision_required_cols_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_outcome_required_cols_chk,
    -- Round-3 M13 sentinel-collision guards.
    DROP CONSTRAINT IF EXISTS audit_outbox_predicted_a_tokens_nonzero_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_predicted_b_tokens_nonzero_chk,
    DROP CONSTRAINT IF EXISTS audit_outbox_predicted_c_tokens_nonzero_chk,
    -- Round-4 M3 outcome cold-start layer guard.
    DROP CONSTRAINT IF EXISTS audit_outbox_cold_start_layer_outcome_chk;

-- Drop indexes (PG drops them transparently with the columns but
-- being explicit makes the rollback intent obvious).
DROP INDEX IF EXISTS audit_outbox_calibration_idx;
DROP INDEX IF EXISTS audit_outbox_tier_idx;
DROP INDEX IF EXISTS audit_outbox_outcome_calibration_idx;
-- Round-3 M7: FK supporting index.
DROP INDEX IF EXISTS audit_outbox_tokenizer_version_id_idx;

-- ============================================================================
-- Drop pre-created partitions. Round-3 fix M4 + round-4 fix B6 + round-5
-- fix N12-B: per-partition row-count guard with ACCESS EXCLUSIVE LOCK,
-- LOCK + COUNT + DROP collapsed into ONE DO $$ block so the three
-- operations share a single implicit transaction even under the autocommit
-- runner. EXECUTE 'DROP TABLE ...' is required because DROP TABLE is not
-- legal inside a PL/pgSQL block as a static statement. Manual data
-- migration required first if any partition has rows.
-- ============================================================================
DO $$
DECLARE
    rc BIGINT;
BEGIN
    -- Round-5 N12-B: LOCK + COUNT + EXECUTE 'DROP TABLE' all inside one DO
    -- block. The ACCESS EXCLUSIVE lock held by the DO block's implicit
    -- transaction blocks all concurrent reads + writes from
    -- LOCK acquisition until END $$ commits, so the count → drop sequence
    -- cannot be interleaved with a concurrent INSERT.
    LOCK TABLE audit_outbox_2026_10 IN ACCESS EXCLUSIVE MODE;
    SELECT COUNT(*) INTO rc FROM audit_outbox_2026_10;
    IF rc > 0 THEN
        RAISE EXCEPTION 'audit_outbox_2026_10 has % rows; refusing to drop. Manual data migration required first.', rc;
    END IF;
    EXECUTE 'DROP TABLE IF EXISTS audit_outbox_2026_10';
END $$;

DO $$
DECLARE
    rc BIGINT;
BEGIN
    LOCK TABLE audit_outbox_2026_09 IN ACCESS EXCLUSIVE MODE;
    SELECT COUNT(*) INTO rc FROM audit_outbox_2026_09;
    IF rc > 0 THEN
        RAISE EXCEPTION 'audit_outbox_2026_09 has % rows; refusing to drop. Manual data migration required first.', rc;
    END IF;
    EXECUTE 'DROP TABLE IF EXISTS audit_outbox_2026_09';
END $$;

DO $$
DECLARE
    rc BIGINT;
BEGIN
    LOCK TABLE audit_outbox_2026_08 IN ACCESS EXCLUSIVE MODE;
    SELECT COUNT(*) INTO rc FROM audit_outbox_2026_08;
    IF rc > 0 THEN
        RAISE EXCEPTION 'audit_outbox_2026_08 has % rows; refusing to drop. Manual data migration required first.', rc;
    END IF;
    EXECUTE 'DROP TABLE IF EXISTS audit_outbox_2026_08';
END $$;

-- Drop the 18 prediction columns.
ALTER TABLE audit_outbox
    DROP COLUMN IF EXISTS predicted_a_tokens,
    DROP COLUMN IF EXISTS predicted_b_tokens,
    DROP COLUMN IF EXISTS predicted_c_tokens,
    DROP COLUMN IF EXISTS reserved_strategy,
    DROP COLUMN IF EXISTS prediction_strategy_used,
    DROP COLUMN IF EXISTS prediction_policy_used,
    DROP COLUMN IF EXISTS tokenizer_tier,
    DROP COLUMN IF EXISTS tokenizer_version_id,
    DROP COLUMN IF EXISTS prediction_confidence,
    DROP COLUMN IF EXISTS prediction_sample_size,
    DROP COLUMN IF EXISTS cold_start_layer_used,
    DROP COLUMN IF EXISTS run_projection_at_decision_atomic,
    DROP COLUMN IF EXISTS run_predicted_remaining_steps,
    DROP COLUMN IF EXISTS run_steps_completed_so_far,
    DROP COLUMN IF EXISTS actual_input_tokens,
    DROP COLUMN IF EXISTS actual_output_tokens,
    DROP COLUMN IF EXISTS delta_b_ratio,
    DROP COLUMN IF EXISTS delta_c_ratio;
