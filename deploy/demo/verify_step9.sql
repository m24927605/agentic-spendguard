-- Phase 2B Step 9 verification (invoice-mode).
-- After: reserve(100) -> commit_estimated(42) -> provider_report(38) -> invoice_reconcile(40)
-- Expected:
--   available_budget = 460 (= 462 from step 8 - 2 from invoice delta=+2)
--   reserved_hold    = 0
--   committed_spend  = 40 (= 38 from step 8 + 2 from invoice delta)
--   adjustment       = 500 (seed unchanged)
--   commits.latest_state = 'invoice_reconciled'
--   commits.estimated_amount = 42
--   commits.provider_reported_amount = 38
--   commits.invoice_reconciled_amount = 40
--   commits.delta_to_reserved = 40 - 100 = -60
--
-- audit_outbox events:
--   audit.decision: 4 (deposit + reserve + provider_report + invoice_reconcile)
--   audit.outcome:  2 (commit_estimated + invoice_reconcile)
--
-- POC limits documented:
--   - invoice <= original_reserved (overrun_debt path deferred)
--   - tolerance_micros interpreted as 0 atomic for token unit
--   - signing_key_id 'ledger-server-mint:v1' POC sentinel; production needs
--     real signing or sentinel skip in CI verifier

\echo
\echo === ledger_transactions: operation_kind counts ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN ('reserve','commit_estimated','provider_report','invoice_reconcile','adjustment')
 GROUP BY operation_kind
 ORDER BY operation_kind;

\echo
\echo === reservations.current_state ===
SELECT current_state, COUNT(*)::int AS n
  FROM reservations
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND budget_id = '44444444-4444-4444-8444-444444444444'
 GROUP BY current_state
 ORDER BY current_state;

\echo
\echo === commits projection (after InvoiceReconcile) ===
SELECT latest_state, estimated_amount_atomic,
       provider_reported_amount_atomic, invoice_reconciled_amount_atomic,
       delta_to_reserved_atomic
  FROM commits
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
 ORDER BY created_at DESC
 LIMIT 5;

\echo
\echo === per-account net (entries-derived) ===
SELECT la.account_kind,
       COALESCE(
         SUM(CASE WHEN le.direction='debit' THEN le.amount_atomic
                  WHEN le.direction='credit' THEN -le.amount_atomic END),
         0)::TEXT AS net_debit
  FROM ledger_entries le
  JOIN ledger_accounts la ON le.ledger_account_id = la.ledger_account_id
 WHERE le.tenant_id = '00000000-0000-4000-8000-000000000001'
   AND la.budget_id = '44444444-4444-4444-8444-444444444444'
   AND le.window_instance_id = '55555555-5555-4555-8555-555555555555'
   AND la.unit_id = '66666666-6666-4666-8666-666666666666'
 GROUP BY la.account_kind
 ORDER BY la.account_kind;

\echo
\echo === audit_outbox event_type counts ===
SELECT event_type, COUNT(*)::int AS n
  FROM audit_outbox
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
 GROUP BY event_type
 ORDER BY event_type;

\echo
\echo === audit_outbox_global_keys idempotency_key suffixes for invoice_reconcile ===
SELECT idempotency_key, event_type
  FROM audit_outbox_global_keys
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind = 'invoice_reconcile'
 ORDER BY event_type;

\echo
\echo === ASSERTIONS ===
DO $$
DECLARE
    v_reserve_count       INT;
    v_commit_count        INT;
    v_provider_count      INT;
    v_invoice_count       INT;
    v_available_net       NUMERIC;
    v_reserved_hold_net   NUMERIC;
    v_committed_spend_net NUMERIC;
    v_committed_state_n   INT;
    v_decision_audit_n    INT;
    v_outcome_audit_n     INT;
    v_invoice_decision_global_n INT;
    v_invoice_outcome_global_n  INT;
    v_commit_state        TEXT;
    v_commit_estimated    NUMERIC;
    v_commit_provider     NUMERIC;
    v_commit_invoice      NUMERIC;
    v_commit_delta        NUMERIC;
BEGIN
    SELECT COUNT(*) INTO v_reserve_count
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'reserve';
    IF v_reserve_count <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 reserve tx; got %', v_reserve_count;
    END IF;

    SELECT COUNT(*) INTO v_commit_count
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'commit_estimated';
    IF v_commit_count <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 commit_estimated tx; got %', v_commit_count;
    END IF;

    SELECT COUNT(*) INTO v_provider_count
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'provider_report';
    IF v_provider_count <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 provider_report tx; got %', v_provider_count;
    END IF;

    SELECT COUNT(*) INTO v_invoice_count
      FROM ledger_transactions
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'invoice_reconcile';
    IF v_invoice_count <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 invoice_reconcile tx; got %', v_invoice_count;
    END IF;

    SELECT
        COALESCE(SUM(CASE WHEN la.account_kind = 'available_budget'
                          THEN (CASE WHEN le.direction='debit' THEN le.amount_atomic
                                     ELSE -le.amount_atomic END)
                          ELSE 0 END), 0),
        COALESCE(SUM(CASE WHEN la.account_kind = 'reserved_hold'
                          THEN (CASE WHEN le.direction='debit' THEN le.amount_atomic
                                     ELSE -le.amount_atomic END)
                          ELSE 0 END), 0),
        COALESCE(SUM(CASE WHEN la.account_kind = 'committed_spend'
                          THEN (CASE WHEN le.direction='debit' THEN le.amount_atomic
                                     ELSE -le.amount_atomic END)
                          ELSE 0 END), 0)
      INTO v_available_net, v_reserved_hold_net, v_committed_spend_net
      FROM ledger_entries le
      JOIN ledger_accounts la ON le.ledger_account_id = la.ledger_account_id
     WHERE le.tenant_id = '00000000-0000-4000-8000-000000000001'
       AND la.budget_id = '44444444-4444-4444-8444-444444444444'
       AND le.window_instance_id = '55555555-5555-4555-8555-555555555555'
       AND la.unit_id = '66666666-6666-4666-8666-666666666666';

    -- Sign convention: net_debit is signed. balance = -net_debit.
    IF -v_available_net <> 460 THEN
        RAISE EXCEPTION 'EXPECTED available_budget balance 460; got %', -v_available_net;
    END IF;
    IF -v_reserved_hold_net <> 0 THEN
        RAISE EXCEPTION 'EXPECTED reserved_hold balance 0; got %', -v_reserved_hold_net;
    END IF;
    IF -v_committed_spend_net <> 40 THEN
        RAISE EXCEPTION 'EXPECTED committed_spend balance 40; got %', -v_committed_spend_net;
    END IF;

    SELECT COUNT(*) INTO v_committed_state_n
      FROM reservations
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND budget_id = '44444444-4444-4444-8444-444444444444'
       AND current_state = 'committed';
    IF v_committed_state_n <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 committed reservation; got %', v_committed_state_n;
    END IF;

    SELECT latest_state, estimated_amount_atomic,
           provider_reported_amount_atomic, invoice_reconciled_amount_atomic,
           delta_to_reserved_atomic
      INTO v_commit_state, v_commit_estimated, v_commit_provider,
           v_commit_invoice, v_commit_delta
      FROM commits
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
     ORDER BY created_at DESC
     LIMIT 1;
    IF v_commit_state <> 'invoice_reconciled' THEN
        RAISE EXCEPTION 'EXPECTED commits.latest_state=invoice_reconciled; got %', v_commit_state;
    END IF;
    IF v_commit_estimated <> 42 THEN
        RAISE EXCEPTION 'EXPECTED commits.estimated_amount=42; got %', v_commit_estimated;
    END IF;
    IF v_commit_provider <> 38 THEN
        RAISE EXCEPTION 'EXPECTED commits.provider_reported_amount=38; got %', v_commit_provider;
    END IF;
    IF v_commit_invoice <> 40 THEN
        RAISE EXCEPTION 'EXPECTED commits.invoice_reconciled_amount=40; got %', v_commit_invoice;
    END IF;
    IF v_commit_delta <> -60 THEN
        RAISE EXCEPTION 'EXPECTED commits.delta_to_reserved=-60 (=40-100); got %', v_commit_delta;
    END IF;

    SELECT COUNT(*) INTO v_decision_audit_n
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.decision';
    SELECT COUNT(*) INTO v_outcome_audit_n
      FROM audit_outbox
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND event_type = 'spendguard.audit.outcome';
    -- 4 audit.decision (deposit + reserve + provider_report + invoice_reconcile)
    -- 2 audit.outcome (commit_estimated + invoice_reconcile)
    IF v_decision_audit_n <> 4 THEN
        RAISE EXCEPTION 'EXPECTED 4 audit.decision events; got %', v_decision_audit_n;
    END IF;
    IF v_outcome_audit_n <> 2 THEN
        RAISE EXCEPTION 'EXPECTED 2 audit.outcome events; got %', v_outcome_audit_n;
    END IF;

    -- Step 9 Δ8: dual-row global_keys idempotency suffixes.
    SELECT COUNT(*) INTO v_invoice_decision_global_n
      FROM audit_outbox_global_keys
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'invoice_reconcile'
       AND event_type = 'spendguard.audit.decision'
       AND idempotency_key LIKE '%:decision';
    IF v_invoice_decision_global_n <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 invoice_reconcile decision row in global_keys with :decision suffix; got %',
                        v_invoice_decision_global_n;
    END IF;
    SELECT COUNT(*) INTO v_invoice_outcome_global_n
      FROM audit_outbox_global_keys
     WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
       AND operation_kind = 'invoice_reconcile'
       AND event_type = 'spendguard.audit.outcome'
       AND idempotency_key LIKE '%:outcome';
    IF v_invoice_outcome_global_n <> 1 THEN
        RAISE EXCEPTION 'EXPECTED 1 invoice_reconcile outcome row in global_keys with :outcome suffix; got %',
                        v_invoice_outcome_global_n;
    END IF;

    RAISE NOTICE 'Phase 2B Step 9 assertions PASS (available=460, committed=40, reserved=0, commits.latest_state=invoice_reconciled, delta=-60, dual-row global_keys suffix verified)';
END
$$;
