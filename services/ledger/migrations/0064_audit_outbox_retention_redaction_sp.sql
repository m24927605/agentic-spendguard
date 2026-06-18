-- Retention redaction: sanctioned in-place prompt-data redaction for
-- audit_outbox, gated through a SECURITY DEFINER SP with a SURGICAL
-- immutability-trigger exemption.
--
-- Background (the bug this closes):
--   `audit_outbox.cloudevent_payload` is protected by the
--   `audit_outbox_immutability` trigger (function
--   `reject_audit_outbox_immutable_columns`, migration 0011, re-asserted
--   in 0046 §step 5). That function rejects ANY change to
--   cloudevent_payload with errcode 42P10. The retention sweeper
--   (services/retention_sweeper) must, per the stated compliance
--   guarantee, redact `cloudevent_payload->'data'` once a tenant's
--   prompt_retention_days window elapses. A raw UPDATE is unconditionally
--   rejected, so prompt-text redaction for audit_outbox silently never
--   ran — raw prompt data was retained indefinitely past policy.
--
-- Fix shape (per verifier_note on the High finding — kept surgical so the
-- audit-immutability hole the trigger protects is NOT reopened):
--   1. `retention_redaction_role` — dedicated role; the SP is the ONLY
--      thing it can use to touch cloudevent_payload, and that touch is
--      bounded to the redaction shape below.
--   2. `redact_audit_outbox_data(p_audit_id, p_marker, p_digest_hex)` —
--      SECURITY DEFINER SP that is the SOLE sanctioned redaction path. It
--      sets a transaction-local marker GUC scoped to the exact target row,
--      then performs the bounded jsonb_set, then clears the GUC. It
--      asserts the sanctioned delta itself so a buggy/compromised caller
--      cannot widen the exemption.
--   3. `reject_audit_outbox_immutable_columns()` is CREATE OR REPLACE'd so
--      the EXISTING `audit_outbox_immutability` trigger now dispatches to a
--      body that permits a cloudevent_payload change ONLY when (a) the SP's
--      per-row GUC marks the sanctioned context for THIS row AND (b) the
--      sole delta is `data` -> the redaction marker plus the addition of
--      `_data_sha256_hex`. Every other immutable column must still be
--      unchanged. Anything else — including a direct UPDATE from any role
--      with the GUC unset — is rejected with 42P10 exactly as before.
--
-- This migration is purely additive (CREATE OR REPLACE on the existing
-- trigger function — no trigger DROP, no audit downtime; the trigger keeps
-- firing BEFORE UPDATE and just dispatches to the tightened body) and
-- introduces no fail-open path: with the GUC unset the function is
-- byte-for-byte equivalent to the 0046 body.

-- =====================================================================
-- Step 1: dedicated redaction role.
-- =====================================================================
-- NOINHERIT mirrors ledger_application_role (0012): membership does not
-- silently confer the privilege; the sweeper's connection role is granted
-- EXECUTE explicitly. Idempotent create so re-applying the migration chain
-- (tests, fresh demo) does not error on an existing role.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'retention_redaction_role') THEN
        CREATE ROLE retention_redaction_role NOINHERIT;
    END IF;
END;
$$;

-- =====================================================================
-- Step 2: sanctioned redaction SP.
-- =====================================================================
-- SECURITY DEFINER so it runs with the function-owner's privilege (the
-- migration runner / table owner) regardless of the calling role, which
-- is what lets a low-privilege retention_redaction_role drive exactly this
-- one bounded mutation and nothing else. search_path locked to
-- pg_catalog, pg_temp per the 0046 convention (CVE-2018-1058).
--
-- The SP:
--   * Validates p_marker is the redaction marker shape (_redacted=true).
--   * Validates p_digest_hex is a lowercase 64-char hex SHA-256 digest so
--     a downstream forensic reader gets a well-formed value (NOT an
--     audit-chain continuity anchor — see services/retention_sweeper
--     module header; the chain is anchored by cloudevent_payload_signature).
--   * Sets a transaction-local GUC naming the exact target row, performs
--     the bounded jsonb_set, then clears the GUC. The GUC is set with
--     is_local = true so it is automatically discarded at COMMIT/ROLLBACK
--     and never leaks to a later statement in the same session.
CREATE OR REPLACE FUNCTION redact_audit_outbox_data(
    p_audit_id   UUID,
    p_marker     JSONB,
    p_digest_hex TEXT
) RETURNS VOID
LANGUAGE plpgsql
AS $$
DECLARE
    v_rows INT;
BEGIN
    IF p_marker IS NULL
       OR COALESCE((p_marker->>'_redacted')::BOOLEAN, FALSE) IS DISTINCT FROM TRUE THEN
        RAISE EXCEPTION 'redact_audit_outbox_data: marker must be the redaction marker ({_redacted:true,...})'
            USING ERRCODE = '22023';
    END IF;
    IF p_digest_hex IS NULL OR p_digest_hex !~ '^[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'redact_audit_outbox_data: digest must be a lowercase 64-char hex SHA-256'
            USING ERRCODE = '22023';
    END IF;

    -- Mark the sanctioned context for THIS row only (per-row scoping means
    -- the exemption cannot be reused to mutate a different row in the same
    -- transaction). is_local=true -> discarded at txn end.
    PERFORM pg_catalog.set_config('spendguard.redaction_audit_id', p_audit_id::TEXT, true);

    UPDATE public.audit_outbox
       SET cloudevent_payload =
               jsonb_set(
                   jsonb_set(cloudevent_payload, '{data}', p_marker, true),
                   '{_data_sha256_hex}', to_jsonb(p_digest_hex), true)
     WHERE audit_outbox_id = p_audit_id
       -- Idempotent + bounded: only redact a row whose data is not already
       -- redacted, so a re-run is a no-op rather than re-marking.
       AND COALESCE((cloudevent_payload->'data'->>'_redacted')::BOOLEAN, FALSE) = FALSE;
    GET DIAGNOSTICS v_rows = ROW_COUNT;

    -- Clear the sanctioned context immediately so no subsequent statement
    -- in the same transaction inherits the exemption.
    PERFORM pg_catalog.set_config('spendguard.redaction_audit_id', '', true);

    IF v_rows = 0 THEN
        -- Either the row is gone or already redacted. Not an error: the
        -- sweeper's candidate query already filtered to unredacted rows, so
        -- a zero here is a benign race (concurrent redaction). Surfacing it
        -- as success keeps the caller idempotent; the caller never deletes.
        RETURN;
    END IF;
END;
$$;

ALTER FUNCTION redact_audit_outbox_data(UUID, JSONB, TEXT)
    SECURITY DEFINER SET search_path = pg_catalog, pg_temp;

REVOKE ALL ON FUNCTION redact_audit_outbox_data(UUID, JSONB, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION redact_audit_outbox_data(UUID, JSONB, TEXT)
    TO retention_redaction_role;

COMMENT ON FUNCTION redact_audit_outbox_data IS
    '0064: SOLE sanctioned in-place prompt-data redaction path for audit_outbox. SECURITY DEFINER + REVOKE FROM PUBLIC + GRANT to retention_redaction_role. Sets a per-row txn-local GUC, performs the bounded data->marker + _data_sha256_hex jsonb_set, clears the GUC. The immutability trigger only exempts cloudevent_payload changes carrying this exact shape under this GUC; everything else is still 42P10.';

-- =====================================================================
-- Step 3: tighten the immutability trigger function to surgically exempt
-- the sanctioned redaction shape. CREATE OR REPLACE — no trigger DROP.
-- =====================================================================
-- Semantics vs 0046:
--   * With `spendguard.redaction_audit_id` UNSET (every normal path,
--     every direct UPDATE, every other role) this is byte-for-byte
--     equivalent to the 0046 body: cloudevent_payload is part of the
--     immutable tuple and any change is rejected with 42P10.
--   * The exemption fires ONLY when ALL of:
--       - the GUC equals NEW.audit_outbox_id (sanctioned context, this row)
--       - NEW.cloudevent_payload reconstructs EXACTLY as OLD with data set
--         to the redaction marker and _data_sha256_hex added (nothing else
--         in the payload changed, and the marker really is a redaction
--         marker)
--     in which case cloudevent_payload is EXCLUDED from the immutable
--     tuple comparison while EVERY other immutable column is still
--     compared. A change to any other column alongside the redaction is
--     still rejected.
CREATE OR REPLACE FUNCTION reject_audit_outbox_immutable_columns()
RETURNS TRIGGER
SECURITY INVOKER
SET search_path = pg_catalog, pg_temp
AS $$
DECLARE
    v_sanctioned BOOLEAN := FALSE;
    v_redaction_ctx TEXT;
    v_expected JSONB;
BEGIN
    -- Determine whether this UPDATE is the sanctioned redaction of THIS row.
    v_redaction_ctx := current_setting('spendguard.redaction_audit_id', true);
    IF v_redaction_ctx IS NOT NULL
       AND v_redaction_ctx <> ''
       AND v_redaction_ctx = NEW.audit_outbox_id::TEXT
       AND OLD.cloudevent_payload IS DISTINCT FROM NEW.cloudevent_payload THEN
        -- Reconstruct the ONLY permitted NEW payload from OLD: data -> the
        -- NEW marker (which must itself be a redaction marker) plus the
        -- addition of _data_sha256_hex. If NEW matches this exactly, the
        -- sole delta is the sanctioned redaction shape and nothing else.
        IF COALESCE((NEW.cloudevent_payload->'data'->>'_redacted')::BOOLEAN, FALSE) = TRUE
           AND (NEW.cloudevent_payload ? '_data_sha256_hex') THEN
            v_expected := jsonb_set(
                jsonb_set(OLD.cloudevent_payload,
                          '{data}', NEW.cloudevent_payload->'data', true),
                '{_data_sha256_hex}', NEW.cloudevent_payload->'_data_sha256_hex', true);
            IF v_expected = NEW.cloudevent_payload THEN
                v_sanctioned := TRUE;
            END IF;
        END IF;
    END IF;

    -- Immutable-column comparison. cloudevent_payload is included UNLESS the
    -- sanctioned-redaction exemption fired, in which case it is compared as
    -- OLD-vs-OLD (always equal) so the rest of the tuple is still enforced.
    IF (OLD.audit_outbox_id, OLD.audit_decision_event_id, OLD.decision_id,
        OLD.tenant_id, OLD.ledger_transaction_id, OLD.event_type,
        OLD.cloudevent_payload, OLD.cloudevent_payload_signature,
        OLD.ledger_fencing_epoch, OLD.workload_instance_id,
        OLD.recorded_at, OLD.recorded_month,
        OLD.producer_sequence, OLD.idempotency_key,
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
       (NEW.audit_outbox_id, NEW.audit_decision_event_id, NEW.decision_id,
        NEW.tenant_id, NEW.ledger_transaction_id, NEW.event_type,
        CASE WHEN v_sanctioned THEN OLD.cloudevent_payload
             ELSE NEW.cloudevent_payload END,
        NEW.cloudevent_payload_signature,
        NEW.ledger_fencing_epoch, NEW.workload_instance_id,
        NEW.recorded_at, NEW.recorded_month,
        NEW.producer_sequence, NEW.idempotency_key,
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
        RAISE EXCEPTION 'audit_outbox immutable columns cannot be changed (incl. prediction extension cols)'
            USING ERRCODE = '42P10';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

COMMENT ON FUNCTION reject_audit_outbox_immutable_columns IS
    '0064: audit_outbox immutability with a surgical exemption for the sanctioned retention redaction. With the spendguard.redaction_audit_id GUC unset this is identical to the 0046 body (any cloudevent_payload change -> 42P10). The exemption permits a cloudevent_payload change ONLY when the per-row GUC marks the sanctioned context and the sole delta is data -> redaction marker + _data_sha256_hex; set exclusively by the redact_audit_outbox_data SP.';
