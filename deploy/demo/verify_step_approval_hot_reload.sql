-- =====================================================================
-- Issue #59 slice 4 (#68): DEMO_MODE=approval_hot_reload verify SQL.
--
-- Asserts the hot-reload regression path: an approval that gets
-- resolved=approved BUT whose resume() fired AFTER a bundle rotation
-- between approval and resume MUST NOT be bundled.
--
-- Expected post-conditions:
--   * §A: denied_decision row exists (REQUIRE_APPROVAL emit) but
--         NO new reserve row (resume refused via BUNDLE_HOT_RELOADED).
--   * §B: approval_requests row reached state='approved' BUT
--         bundled_ledger_transaction_id IS NULL.
--   * §C: NO leftover pending rows.
--   * §D: decision_context carries the 4 issue-59 pricing fields AND
--         a non-empty contract_bundle_hash_hex (the one the operator
--         approved against — semantically frozen).
-- =====================================================================

\set ON_ERROR_STOP on

-- §A: denied_decision row exists, NO reserve row from resume.
\echo '[verify-hr] §A: denied_decision row exists; reserve row count = 0 (resume refused)'
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
    RAISE NOTICE '[verify-hr] §A denied_rows=% reserve_rows=%',
                 denied_rows, reserve_rows;
    IF denied_rows < 1 THEN
        RAISE EXCEPTION '§A FAIL: expected at least 1 denied_decision row (from REQUIRE_APPROVAL)';
    END IF;
    IF reserve_rows > 0 THEN
        RAISE EXCEPTION '§A FAIL: expected 0 reserve rows (resume refused via BUNDLE_HOT_RELOADED) but got %',
                        reserve_rows;
    END IF;
END$$;

-- §B: approval_requests row reached state='approved' BUT
--     bundled_ledger_transaction_id IS NULL.
\echo '[verify-hr] §B: approval_requests approved BUT NOT bundled'
DO $$
DECLARE
    approved_unbundled INT;
    approved_and_bundled INT;
BEGIN
    SELECT
        COUNT(*) FILTER (WHERE state = 'approved' AND bundled_ledger_transaction_id IS NULL),
        COUNT(*) FILTER (WHERE state = 'approved' AND bundled_ledger_transaction_id IS NOT NULL)
      INTO approved_unbundled, approved_and_bundled
      FROM approval_requests
     WHERE created_at > now() - interval '5 minutes';
    RAISE NOTICE '[verify-hr] §B approved_unbundled=% approved_and_bundled=%',
                 approved_unbundled, approved_and_bundled;
    IF approved_unbundled < 1 THEN
        RAISE EXCEPTION '§B FAIL: expected at least 1 approved+unbundled approval_requests row';
    END IF;
    IF approved_and_bundled > 0 THEN
        RAISE EXCEPTION '§B FAIL: % approval_requests rows were bundled despite hot-reload — bundle-hash check did not fire',
                        approved_and_bundled;
    END IF;
END$$;

-- §C: No leftover pending approval_requests from the run.
\echo '[verify-hr] §C: no pending approval_requests left from this run'
DO $$
DECLARE
    pending_rows INT;
BEGIN
    SELECT COUNT(*) INTO pending_rows
      FROM approval_requests
     WHERE created_at > now() - interval '5 minutes'
       AND state = 'pending';
    RAISE NOTICE '[verify-hr] §C pending_rows=%', pending_rows;
    IF pending_rows > 0 THEN
        RAISE EXCEPTION '§C FAIL: % approval_requests still pending — control-plane resolve did not transition them',
                        pending_rows;
    END IF;
END$$;

-- §D: decision_context_json carries the 4 issue-59 pricing fields
--     AND contract_bundle_hash_hex (the captured-at-PRE bundle hash).
\echo '[verify-hr] §D: decision_context carries frozen bundle hash + 4 pricing fields'
DO $$
DECLARE
    rows_missing INT;
BEGIN
    SELECT COUNT(*) INTO rows_missing
      FROM approval_requests
     WHERE created_at > now() - interval '5 minutes'
       AND state = 'approved'
       AND NOT (
              decision_context ? 'contract_bundle_hash_hex'
          AND decision_context ? 'pricing_version'
          AND decision_context ? 'price_snapshot_hash_hex'
          AND decision_context ? 'fx_rate_version'
          AND decision_context ? 'unit_conversion_version'
       );
    RAISE NOTICE '[verify-hr] §D rows missing required decision_context fields=%', rows_missing;
    IF rows_missing > 0 THEN
        RAISE EXCEPTION '§D FAIL: % rows missing one or more frozen-at-PRE fields in decision_context_json',
                        rows_missing;
    END IF;
END$$;

\echo '[verify-hr] Issue #59 slice 4 hot-reload regression verification PASS'
