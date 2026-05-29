-- Down-migration: reverse 0015_audit_outcome_quarantine_prediction_columns.sql
-- (round-3 fix B2; round-4 fixes M4 + M5; round-5 fixes N12-A + N14).
--
-- Apply BEFORE 0013_down — the 18 columns mirror canonical_events but live
-- on a different table, so the only ordering constraint is the
-- internal one within this file (round-4 M5): DROP COLUMN must precede
-- CREATE OR REPLACE FUNCTION (see Step 2 rationale).
--
-- Round-3 fix M4 + round-4 fix M4 + round-5 fix N12-A: per-file destructive-down
-- guard (spendguard.allow_destructive_down_0015) — see slice §11 for the
-- exact SET form. SET (not SET LOCAL) because the migration runner
-- autocommits each statement.
-- Round-3 fix M10: no explicit BEGIN/COMMIT.
--
-- Round-4 fix M5: internal step order corrected. Round-3 ran
-- "CREATE OR REPLACE FUNCTION (revert to 24-col)" BEFORE "ALTER TABLE
-- DROP COLUMN (×18)". That created a tamper window: the reverted
-- function references only 24 columns of the OLD/NEW row, but the 18
-- prediction columns still exist on the table.
--
-- Round-5 fix N14: round-4 M5 reversed the order INSIDE the file but the
-- migration runner autocommits each top-level statement, so steps 1, 2,
-- and 3 each ran in their own transaction. Between step 2's autocommit
-- and step 3's CREATE OR REPLACE FUNCTION starting, the trigger function
-- was STILL the 42-column version from the up-migration while the 18
-- columns were already gone — every UPDATE between those statements
-- raised "column does not exist" errors and tampered any in-flight
-- UPDATE. The mirror image of round-3's window: same trigger / column
-- shape mismatch, opposite polarity.
--
-- Round-5 N14 fix: collapse steps 1 + 2 + 3 into ONE DO $$ block. The
-- DROP CONSTRAINTs, DROP COLUMNs, and CREATE OR REPLACE FUNCTION all run
-- via EXECUTE (required because DDL is not legal as a static PL/pgSQL
-- statement). They share one implicit transaction under the autocommit
-- runner, so an UPDATE issued during the down-migration either sees the
-- 42-col function + 42-col table (rolled back to fail under existing
-- trigger semantics) OR the 24-col function + 24-col table (matched);
-- never the column / function mismatch that creates the tamper window.
-- The CREATE OR REPLACE FUNCTION body uses the $body$ ... $body$ tag to
-- avoid colliding with the outer DO $$ ... $$ dollar quotes.

DO $$
BEGIN
    IF current_setting('spendguard.allow_destructive_down_0015', true) IS DISTINCT FROM 'on' THEN
        RAISE EXCEPTION 'destructive down-migration 0015 requires `SET spendguard.allow_destructive_down_0015 = ''on''` first (session-scoped; runner autocommits so SET LOCAL would die at the commit boundary)';
    END IF;
    RAISE NOTICE 'DESTRUCTIVE down-migration 0015 proceeding (caller: %)', current_user;
END $$;

-- ============================================================================
-- Round-5 N14 atomic revert block: drop CHECKs + drop 18 columns + revert
-- trigger function inside ONE DO $$ block so the three steps share one
-- implicit transaction under the autocommit migration runner. Without
-- this collapse, round-4 M5's internal step order was correct on paper
-- but the runner's per-statement autocommit re-introduced a function /
-- column mismatch window between steps 2 and 3.
--
-- DROP COLUMN takes ACCESS EXCLUSIVE on the table; the lock is held until
-- the DO block commits (END $$), so concurrent UPDATEs are blocked for
-- the entire DROP CHECK → DROP COLUMN → CREATE OR REPLACE FUNCTION
-- sequence. After END $$ commits, the table shape and the trigger
-- function's tuple comparison are simultaneously consistent (24 cols
-- both).
-- ============================================================================
DO $$
BEGIN
    -- Step 1: drop the sentinel/domain CHECK constraints. ALTER TABLE
    -- DROP COLUMN below would cascade-drop constraints referencing the
    -- dropped columns, but being explicit is more self-documenting and
    -- resilient to constraint additions on legacy rows.
    EXECUTE $sql$
        ALTER TABLE audit_outcome_quarantine
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_reserved_strategy_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_prediction_strategy_used_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_prediction_policy_used_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_tokenizer_tier_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_prediction_confidence_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_cold_start_layer_used_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_predicted_tokens_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_actual_tokens_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_run_steps_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_run_projection_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_run_projection_int64_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_prediction_sample_size_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_delta_b_ratio_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_delta_c_ratio_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_predicted_b_tokens_nonzero_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_predicted_c_tokens_nonzero_chk,
            DROP CONSTRAINT IF EXISTS audit_outcome_quarantine_cold_start_layer_outcome_chk
    $sql$;

    -- Step 2: drop the 18 prediction columns. ACCESS EXCLUSIVE held by
    -- ALTER TABLE blocks all concurrent UPDATEs through the next step.
    EXECUTE $sql$
        ALTER TABLE audit_outcome_quarantine
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
            DROP COLUMN IF EXISTS delta_c_ratio
    $sql$;

    -- Step 3: revert the trigger function to the pre-SLICE_01 24-col body
    -- from services/canonical_ingest/migrations/0005_immutability_triggers.sql.
    -- The function body uses a $body... $body... dollar tag (instead of the
    -- bare double-dollar) so the inner literal does not collide with the
    -- outer DO block's dollar quote. SECURITY INVOKER + SET search_path
    -- lockdown is the round-5 Security Finding 1 fix (CVE-2018-1058).
    EXECUTE $sql$
        CREATE OR REPLACE FUNCTION reject_quarantine_immutable_columns()
        RETURNS TRIGGER
        SECURITY INVOKER
        SET search_path = pg_catalog, pg_temp
        AS $body$
        BEGIN
            IF (OLD.quarantine_id, OLD.event_id, OLD.tenant_id, OLD.decision_id,
                OLD.storage_class, OLD.producer_id, OLD.producer_sequence,
                OLD.producer_signature, OLD.signing_key_id, OLD.schema_bundle_id,
                OLD.schema_bundle_hash, OLD.event_type, OLD.specversion, OLD.source,
                OLD.event_time, OLD.datacontenttype, OLD.payload_json,
                OLD.payload_blob_ref, OLD.region_id, OLD.ingest_shard_id,
                OLD.ingest_log_offset, OLD.run_id, OLD.quarantined_at,
                OLD.orphan_after)
               IS DISTINCT FROM
               (NEW.quarantine_id, NEW.event_id, NEW.tenant_id, NEW.decision_id,
                NEW.storage_class, NEW.producer_id, NEW.producer_sequence,
                NEW.producer_signature, NEW.signing_key_id, NEW.schema_bundle_id,
                NEW.schema_bundle_hash, NEW.event_type, NEW.specversion, NEW.source,
                NEW.event_time, NEW.datacontenttype, NEW.payload_json,
                NEW.payload_blob_ref, NEW.region_id, NEW.ingest_shard_id,
                NEW.ingest_log_offset, NEW.run_id, NEW.quarantined_at,
                NEW.orphan_after) THEN
                RAISE EXCEPTION 'audit_outcome_quarantine immutable columns cannot be changed'
                    USING ERRCODE = '42P10';
            END IF;
            IF OLD.state = 'released' OR OLD.state = 'orphaned' THEN
                RAISE EXCEPTION 'audit_outcome_quarantine state is terminal'
                    USING ERRCODE = '42P10';
            END IF;
            RETURN NEW;
        END;
        $body$ LANGUAGE plpgsql;
    $sql$;
END $$;
