-- Slice 6 verification queries for `DEMO_MODE=litellm_real` (steps 1+2
-- of the 4-step demo per ACCEPTANCE.md §5.1). Asserts that the LiteLLM
-- proxy callback drove a complete reserve→commit lifecycle for the
-- ALLOW step AND a deny path for the DENY step (no commit).
--
-- Ground-truth schema (verified by Slice 1 R3 Staff panel, see
-- slice-06.md inherited-findings table):
--   canonical_events: payload_json column has a `data_b64` field
--     containing the base64-encoded CloudEvent body; event_type is the
--     CloudEvent type string (e.g. "spendguard.audit.decision").
--   ledger_transactions.operation_kind IN ('reserve','commit_estimated',
--     'invoice_reconcile','denied_decision').
--   Time columns are `event_time` / `ingest_at` (NOT `recorded_at`).

\echo
\echo === ledger_transactions: operation_kind counts (litellm_real) ===
SELECT operation_kind, COUNT(*)::int AS n
  FROM ledger_transactions
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND operation_kind IN (
     'reserve', 'commit_estimated', 'denied_decision'
   )
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
\echo === canonical_events.event_type counts (litellm_real) ===
SELECT event_type, COUNT(*)::int AS n
  FROM canonical_events
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
   AND event_type IN (
     'spendguard.audit.decision',
     'spendguard.audit.outcome'
   )
 GROUP BY event_type
 ORDER BY event_type;

\echo
\echo === ALLOW step: commit row for the demo's ALLOW call ===
SELECT latest_state, estimated_amount_atomic
  FROM commits
 WHERE tenant_id = '00000000-0000-4000-8000-000000000001'
 ORDER BY created_at DESC
 LIMIT 5;

\echo
\echo === per-account net (entries-derived; ALLOW step committed) ===
SELECT la.account_kind,
       COALESCE(
         SUM(CASE WHEN le.direction='debit'  THEN le.amount_atomic
                  WHEN le.direction='credit' THEN -le.amount_atomic END),
         0)::TEXT AS net_atomic
  FROM ledger_entries le
  JOIN ledger_accounts la ON la.account_id = le.account_id
 WHERE la.tenant_id = '00000000-0000-4000-8000-000000000001'
   AND la.budget_id = '44444444-4444-4444-8444-444444444444'
 GROUP BY la.account_kind
 ORDER BY la.account_kind;
