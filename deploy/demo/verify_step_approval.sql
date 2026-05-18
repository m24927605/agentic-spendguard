-- =====================================================================
-- Round-2 #9 part 2: DEMO_MODE=approval closed-loop verify SQL.
--
-- Asserts the post-conditions after the demo runner exercises the
-- full REQUIRE_APPROVAL → control-plane resolve → ResumeAfterApproval
-- round-trip:
--   1. Sidecar's contract evaluator emits REQUIRE_APPROVAL on the
--      demo's 500_000_000 claim (the bundle's `require-approval-large`
--      rule fires on claim_amount_atomic_gt 400_000_000).
--   2. post_approval_required_decision SP (migration 0037) writes a
--      'pending' approval_requests row with non-empty
--      decision_context + requested_effect JSON.
--   3. Demo runner POSTs target_state=approved to control-plane's
--      /v1/approvals/:id/resolve — approval row transitions
--      pending → approved.
--   4. SDK e.resume(client) calls ResumeAfterApproval; sidecar
--      reads the resolved row, calls Ledger.ReserveSet, then
--      MarkApprovalBundled atomically links the approval row to
--      the new ledger_transaction.
--
-- Asserts:
--   * §A: at least one spendguard.audit.decision row exists
--   * §B: approval_requests row reached state='approved' with
--         non-empty decision_context + requested_effect AND
--         bundled_ledger_transaction_id IS NOT NULL
--   * §C: NO 'pending' approval_requests row left over from the run
--   * §D: ledger_transactions row corresponding to the bundled
--         resume reservation exists (operation_kind='reserve')
-- =====================================================================

\set ON_ERROR_STOP on

-- §A: ledger_transactions has both a denied_decision row (from the
-- post_approval_required_decision SP wrapping post_denied_decision_transaction)
-- AND a reserve row (from the resume path's Ledger.ReserveSet call).
\echo '[verify] §A: denied_decision + reserve rows exist for the demo run'
DO $$
DECLARE
    denied_rows INT;
    reserve_rows INT;
BEGIN
    SELECT
        COUNT(*) FILTER (WHERE operation_kind = 'denied_decision'),
        COUNT(*) FILTER (WHERE operation_kind = 'reserve')
      INTO denied_rows, reserve_rows
      FROM ledger_transactions
     WHERE recorded_at > now() - interval '5 minutes';
    RAISE NOTICE '[verify] §A denied_rows=% reserve_rows=%',
                 denied_rows, reserve_rows;
    IF denied_rows < 1 THEN
        RAISE EXCEPTION '§A FAIL: expected at least 1 denied_decision row (from REQUIRE_APPROVAL)';
    END IF;
    IF reserve_rows < 1 THEN
        RAISE EXCEPTION '§A FAIL: expected at least 1 reserve row (from resume ReserveSet)';
    END IF;
END$$;

-- §B: approval_requests row reached state='approved' with non-empty
--     decision_context + requested_effect AND bundled_ledger_transaction_id NOT NULL.
\echo '[verify] §B: approval_requests row approved + bundled'
DO $$
DECLARE
    approved_rows INT;
    sample_id TEXT;
    sample_bundled_tx TEXT;
    decision_context_bytes INT;
    requested_effect_bytes INT;
BEGIN
    SELECT
        COUNT(*),
        MAX(approval_id::text),
        MAX(bundled_ledger_transaction_id::text),
        COALESCE(MAX(LENGTH(decision_context::text)), 0),
        COALESCE(MAX(LENGTH(requested_effect::text)), 0)
      INTO approved_rows, sample_id, sample_bundled_tx,
           decision_context_bytes, requested_effect_bytes
      FROM approval_requests
     WHERE created_at > now() - interval '5 minutes'
       AND state = 'approved'
       AND bundled_ledger_transaction_id IS NOT NULL;
    RAISE NOTICE '[verify] §B approved_bundled_rows=% sample_approval=% sample_bundled_tx=% decision_context_bytes=% requested_effect_bytes=%',
                 approved_rows, sample_id, sample_bundled_tx,
                 decision_context_bytes, requested_effect_bytes;
    IF approved_rows < 1 THEN
        RAISE EXCEPTION '§B FAIL: expected at least 1 approval_requests row state=approved AND bundled_ledger_transaction_id IS NOT NULL';
    END IF;
    IF decision_context_bytes < 50 OR requested_effect_bytes < 50 THEN
        RAISE EXCEPTION '§B FAIL: decision_context (% B) or requested_effect (% B) too short — producer SP did not capture them',
                        decision_context_bytes, requested_effect_bytes;
    END IF;
END$$;

-- §B+: Issue #59 frozen-at-PRE pricing — decision_context_json must
-- carry the 4 pricing fields captured at REQUIRE_APPROVAL time.
\echo '[verify] §B+: decision_context carries the 4 issue-59 pricing fields'
DO $$
DECLARE
    rows_missing_fields INT;
BEGIN
    SELECT COUNT(*) INTO rows_missing_fields
      FROM approval_requests
     WHERE created_at > now() - interval '5 minutes'
       AND state = 'approved'
       AND NOT (
              decision_context ? 'pricing_version'
          AND decision_context ? 'price_snapshot_hash_hex'
          AND decision_context ? 'fx_rate_version'
          AND decision_context ? 'unit_conversion_version'
       );
    RAISE NOTICE '[verify] §B+ rows missing issue-59 pricing fields=%', rows_missing_fields;
    IF rows_missing_fields > 0 THEN
        RAISE EXCEPTION '§B+ FAIL: % approval_requests rows missing one or more of pricing_version/price_snapshot_hash_hex/fx_rate_version/unit_conversion_version in decision_context_json',
                        rows_missing_fields;
    END IF;
END$$;

-- §C: No leftover pending approval_requests from the run.
\echo '[verify] §C: no pending approval_requests left from this run'
DO $$
DECLARE
    pending_rows INT;
BEGIN
    SELECT COUNT(*) INTO pending_rows
      FROM approval_requests
     WHERE created_at > now() - interval '5 minutes'
       AND state = 'pending';
    RAISE NOTICE '[verify] §C pending_rows=%', pending_rows;
    IF pending_rows > 0 THEN
        RAISE EXCEPTION '§C FAIL: % approval_requests still pending — control-plane resolve did not transition them',
                        pending_rows;
    END IF;
END$$;

-- §D: ledger_transactions has a row matching the bundled resume reservation.
\echo '[verify] §D: bundled ledger_transactions row exists'
DO $$
DECLARE
    bundled_tx_id UUID;
    found_in_ledger INT;
BEGIN
    SELECT bundled_ledger_transaction_id
      INTO bundled_tx_id
      FROM approval_requests
     WHERE created_at > now() - interval '5 minutes'
       AND state = 'approved'
       AND bundled_ledger_transaction_id IS NOT NULL
     ORDER BY created_at DESC
     LIMIT 1;
    IF bundled_tx_id IS NULL THEN
        RAISE EXCEPTION '§D FAIL: no approved+bundled approval to cross-check';
    END IF;
    SELECT COUNT(*) INTO found_in_ledger
      FROM ledger_transactions
     WHERE ledger_transaction_id = bundled_tx_id;
    RAISE NOTICE '[verify] §D bundled_tx_id=% found_in_ledger=%',
                 bundled_tx_id, found_in_ledger;
    IF found_in_ledger != 1 THEN
        RAISE EXCEPTION '§D FAIL: bundled_ledger_transaction_id % not found in ledger_transactions',
                        bundled_tx_id;
    END IF;
END$$;

\echo '[verify] Round-2 #9 part 2 closed-loop verification PASS'
