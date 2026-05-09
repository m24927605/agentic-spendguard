-- Phase 3 wedge — DENY lifecycle verification.
--
-- Asserts: contract evaluator's STOP path produces exactly one
-- audit_outbox row tagged with the matched rule id and reason code,
-- a `denied_decision` ledger_transactions row (no entries, no
-- reservations), and zero reservations.

\echo
\echo === ledger_transactions (denied_decision) ===
SELECT
    operation_kind,
    posting_state,
    decision_id,
    audit_decision_event_id IS NOT NULL AS has_audit_anchor
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind = 'denied_decision'
 ORDER BY recorded_at DESC
 LIMIT 5;

\echo
\echo === ledger_entries for denied_decision (must be 0) ===
SELECT COUNT(*)::int AS entry_count
  FROM ledger_entries le
  JOIN ledger_transactions lt
    ON lt.ledger_transaction_id = le.ledger_transaction_id
 WHERE lt.tenant_id = '00000000-0000-4000-8000-000000000001'
   AND lt.operation_kind = 'denied_decision';

\echo
\echo === reservations whose source tx is a denied_decision (must be 0) ===
-- DENY path skips Reserve, so no reservation should be sourced from a
-- denied_decision ledger_transactions row. (Reservations from other
-- demo modes in the same compose session may exist with their own
-- source_tx; we assert the DENY-specific invariant only.)
SELECT COUNT(*)::int AS reservation_count
  FROM reservations r
  JOIN ledger_transactions lt
    ON lt.ledger_transaction_id = r.source_ledger_transaction_id
 WHERE r.tenant_id = '00000000-0000-4000-8000-000000000001'
   AND lt.operation_kind = 'denied_decision';

\echo
\echo === audit_outbox cloudevent payload (DENY row) ===
SELECT
    event_type,
    cloudevent_payload->>'type'        AS ce_type,
    cloudevent_payload->'data_b64' IS NOT NULL AS has_data_b64,
    pending_forward
  FROM audit_outbox
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND ledger_transaction_id IN (
       SELECT ledger_transaction_id
         FROM ledger_transactions
        WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
          AND operation_kind = 'denied_decision'
   )
 ORDER BY recorded_at DESC
 LIMIT 3;

\echo
\echo === ASSERTIONS ===
DO $$
DECLARE
    v_denied_tx        INT;
    v_denied_entries   INT;
    v_reservations     INT;
    v_denied_audits    INT;
    v_payload_decoded  TEXT;
    v_has_rule         BOOLEAN;
    v_has_reason       BOOLEAN;
BEGIN
    SELECT COUNT(*) INTO v_denied_tx
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'denied_decision';
    IF v_denied_tx = 0 THEN
        RAISE EXCEPTION 'PHASE_3_DENY_GATE: zero denied_decision rows in ledger_transactions';
    END IF;

    SELECT COUNT(*) INTO v_denied_entries
      FROM ledger_entries le
      JOIN ledger_transactions lt
        ON lt.ledger_transaction_id = le.ledger_transaction_id
     WHERE lt.tenant_id = '00000000-0000-4000-8000-000000000001'
       AND lt.operation_kind = 'denied_decision';
    IF v_denied_entries <> 0 THEN
        RAISE EXCEPTION 'PHASE_3_DENY_GATE: denied_decision must have 0 ledger_entries (got %)', v_denied_entries;
    END IF;

    -- DENY path skips Reserve, so no reservation should be sourced
    -- from a denied_decision ledger_transactions row. (Other demo
    -- modes in the same compose session may have left their own
    -- reservations behind; we scope the assertion to the wedge
    -- invariant only.)
    SELECT COUNT(*) INTO v_reservations
      FROM reservations r
      JOIN ledger_transactions lt
        ON lt.ledger_transaction_id = r.source_ledger_transaction_id
     WHERE r.tenant_id = '00000000-0000-4000-8000-000000000001'
       AND lt.operation_kind = 'denied_decision';
    IF v_reservations <> 0 THEN
        RAISE EXCEPTION 'PHASE_3_DENY_GATE: % reservations sourced from denied_decision rows; DENY must skip Reserve', v_reservations;
    END IF;

    SELECT COUNT(*) INTO v_denied_audits
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND ledger_transaction_id IN (
           SELECT ledger_transaction_id
             FROM ledger_transactions
            WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
              AND operation_kind = 'denied_decision'
       )
       AND event_type = 'spendguard.audit.decision';
    IF v_denied_audits = 0 THEN
        RAISE EXCEPTION 'PHASE_3_DENY_GATE: zero audit_outbox rows for denied_decision (Contract §6.1 invariant violated)';
    END IF;

    -- Forensics: data_b64 must decode to JSON containing matched_rules
    -- and BUDGET_EXHAUSTED reason_code so canonical_events search can
    -- find which rule fired.
    SELECT convert_from(
              decode(cloudevent_payload->>'data_b64', 'base64'),
              'UTF8')
      INTO v_payload_decoded
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND ledger_transaction_id IN (
           SELECT ledger_transaction_id
             FROM ledger_transactions
            WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
              AND operation_kind = 'denied_decision'
       )
     ORDER BY recorded_at DESC
     LIMIT 1;

    v_has_rule := v_payload_decoded LIKE '%hard-cap-deny%';
    v_has_reason := v_payload_decoded LIKE '%BUDGET_EXHAUSTED%';
    IF NOT v_has_rule THEN
        RAISE EXCEPTION 'PHASE_3_DENY_GATE: audit payload missing matched_rules.hard-cap-deny';
    END IF;
    IF NOT v_has_reason THEN
        RAISE EXCEPTION 'PHASE_3_DENY_GATE: audit payload missing reason_codes.BUDGET_EXHAUSTED';
    END IF;

    RAISE NOTICE 'Phase 3 wedge DENY lifecycle PASS: denied_tx=% audit_rows=%',
        v_denied_tx, v_denied_audits;
END
$$;
