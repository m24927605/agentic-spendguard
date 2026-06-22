-- verify_step_deny_conformance.sql
-- Durable ledger gate for DEMO_MODE=deny_conformance (the deny-conformance harness).
--
-- The harness drives every IN-IMAGE gating adapter (pydantic_ai, langchain,
-- openai_agents, agt, litellm) through a 2B budget-busting claim. The runner's
-- exit code is the per-adapter completeness gate (it fails the run if any
-- adapter does not raise DecisionDenied before its provider call). Here we
-- assert the DURABLE ledger state the run leaves behind on a fresh demo seed,
-- reusing the proven Phase-3 DENY invariants (verify_step_deny.sql) but scaled
-- to the multi-deny harness:
--   * >= 1 denied_decision ledger_transactions row  (sidecar RecordDeniedDecision fired)
--   * each denied_decision row has 0 ledger_entries  (a DENY posts nothing)
--   * 0 reservations sourced from a denied_decision   (hard-cap DENY skips Reserve)
--   * each denied_decision has a spendguard.audit.decision audit_outbox row whose
--     decoded payload carries matched rule `hard-cap-deny` + reason `BUDGET_EXHAUSTED`

\echo
\echo === ledger_transactions: denied_decision rows (deny_conformance) ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind = 'denied_decision'
 GROUP BY operation_kind;

\echo
\echo === ASSERT: every denied_decision is a clean, audited, reservation-free DENY ===
DO $$
DECLARE
    v_denied_tx       INT;
    v_denied_entries  INT;
    v_reservations    INT;
    v_denied_audits   INT;
    v_payload_decoded TEXT;
BEGIN
    SELECT COUNT(*) INTO v_denied_tx
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'denied_decision';
    IF v_denied_tx < 1 THEN
        RAISE EXCEPTION 'DENY_CONFORMANCE_GATE: expected >= 1 denied_decision row, got % (no adapter''s gate fired)', v_denied_tx;
    END IF;

    -- A DENY posts no ledger entries.
    SELECT COUNT(*) INTO v_denied_entries
      FROM ledger_entries le
      JOIN ledger_transactions lt
        ON lt.ledger_transaction_id = le.ledger_transaction_id
     WHERE lt.tenant_id = '00000000-0000-4000-8000-000000000001'
       AND lt.operation_kind = 'denied_decision';
    IF v_denied_entries <> 0 THEN
        RAISE EXCEPTION 'DENY_CONFORMANCE_GATE: denied_decision rows must have 0 ledger_entries (got %)', v_denied_entries;
    END IF;

    -- A hard-cap DENY short-circuits Reserve: no reservation may be sourced
    -- from a denied_decision row. (deny_conformance runs ONLY deny shims in
    -- its compose session, so the harness leaves no reservations at all; this
    -- scoped form mirrors verify_step_deny.sql and stays correct regardless.)
    SELECT COUNT(*) INTO v_reservations
      FROM reservations r
      JOIN ledger_transactions lt
        ON lt.ledger_transaction_id = r.source_ledger_transaction_id
     WHERE r.tenant_id = '00000000-0000-4000-8000-000000000001'
       AND lt.operation_kind = 'denied_decision';
    IF v_reservations <> 0 THEN
        RAISE EXCEPTION 'DENY_CONFORMANCE_GATE: % reservations sourced from denied_decision rows; a hard-cap DENY must skip Reserve', v_reservations;
    END IF;

    -- Contract §6.1: every DENY emits an audit decision CloudEvent.
    SELECT COUNT(*) INTO v_denied_audits
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.decision'
       AND ledger_transaction_id IN (
           SELECT ledger_transaction_id
             FROM ledger_transactions
            WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
              AND operation_kind = 'denied_decision'
       );
    IF v_denied_audits = 0 THEN
        RAISE EXCEPTION 'DENY_CONFORMANCE_GATE: zero audit_outbox decision rows for denied_decision (Contract §6.1 violated)';
    END IF;

    -- Forensics: at least one decoded payload carries the matched rule +
    -- the BUDGET_EXHAUSTED reason code, proving the hard-cap rule fired.
    SELECT convert_from(decode(cloudevent_payload->>'data_b64', 'base64'), 'UTF8')
      INTO v_payload_decoded
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.decision'
       AND ledger_transaction_id IN (
           SELECT ledger_transaction_id
             FROM ledger_transactions
            WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
              AND operation_kind = 'denied_decision'
       )
       AND convert_from(decode(cloudevent_payload->>'data_b64', 'base64'), 'UTF8') LIKE '%hard-cap-deny%'
     ORDER BY recorded_at DESC
     LIMIT 1;

    IF v_payload_decoded IS NULL THEN
        RAISE EXCEPTION 'DENY_CONFORMANCE_GATE: no denied_decision audit payload mentions matched rule hard-cap-deny';
    END IF;
    IF v_payload_decoded NOT LIKE '%BUDGET_EXHAUSTED%' THEN
        RAISE EXCEPTION 'DENY_CONFORMANCE_GATE: hard-cap-deny audit payload missing reason_code BUDGET_EXHAUSTED';
    END IF;

    RAISE NOTICE 'DENY_CONFORMANCE LEDGER OK: denied_tx=% audit_rows=% (clean, audited, reservation-free)',
        v_denied_tx, v_denied_audits;
END;
$$;

\echo
