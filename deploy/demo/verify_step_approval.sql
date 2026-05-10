-- =====================================================================
-- Round-2 #9 part 2 PR 9e: DEMO_MODE=approval verify SQL.
--
-- Asserts the post-conditions after the demo runner exercises the
-- REQUIRE_APPROVAL → ApprovalRequired → e.resume(client) round-trip.
--
-- TODAY (PR 9e shipped, producer-side SP not yet wired):
--   * approval_requests row in 'pending' state — passes
--   * audit_outbox decision row exists with decision='REQUIRE_APPROVAL' — passes
--   * No ledger_transactions row tied to the approval — expected (resume
--     surface returns ApprovalLapsedError until producer SP lands)
--
-- WHEN producer-side post_approval_required_decision SP ships:
--   * Update §B to require approval_requests.state='approved'
--   * Update §C to require approval_requests.bundled_ledger_transaction_id
--     IS NOT NULL after resume
--   * Update §D to require a Continue audit_outbox row tied to the
--     resume idempotency_key (sha256("resume:" + approval_id))
-- =====================================================================

-- §A: At least one decision row exists for the demo run.
\echo '[verify] §A: decision row exists for the demo claim'
SELECT
    COUNT(*) AS decision_rows,
    bool_or(operation_kind = 'spendguard.audit.decision') AS has_decision_kind
FROM ledger_transactions
WHERE recorded_at > now() - interval '5 minutes';

-- §B: approval_requests has at least one pending row from the run.
-- (Once producer SP lands this also asserts decision_context_json /
-- requested_effect_json are non-empty.)
\echo '[verify] §B: approval_requests has a pending row'
SELECT
    COUNT(*) AS pending_approvals,
    COALESCE(MAX(LENGTH(decision_context::text)), 0) AS decision_context_bytes,
    COALESCE(MAX(LENGTH(requested_effect::text)), 0) AS requested_effect_bytes
FROM approval_requests
WHERE created_at > now() - interval '5 minutes'
  AND state = 'pending';

-- §C: bundled_ledger_transaction_id is currently NULL — expected
-- until producer SP wiring lands.
\echo '[verify] §C: bundled_ledger_transaction_id IS NULL (expected today)'
SELECT
    COUNT(*) AS unbundled_pending,
    COALESCE(MAX(approval_id::text), '<none>') AS sample_approval_id
FROM approval_requests
WHERE created_at > now() - interval '5 minutes'
  AND state = 'pending'
  AND bundled_ledger_transaction_id IS NULL;

-- §D: When the producer SP ships, this query should return >= 1 row
-- with operation_kind='spendguard.audit.outcome.resume_continue'.
-- Today it returns 0 (no resume yet); commented to keep the verify
-- output noise-free.
--
-- \echo '[verify] §D (deferred): resume continuation audit_outbox row'
-- SELECT COUNT(*) FROM audit_outbox
-- WHERE cloudevent_type = 'spendguard.audit.outcome.resume_continue'
--   AND recorded_at > now() - interval '5 minutes';
