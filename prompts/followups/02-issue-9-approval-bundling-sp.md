# Followup #9 — Approval bundling SP (S14-followup)

GitHub issue: https://github.com/m24927605/agentic-flow-cost-evaluation/issues/9

## Goal

Close the `REQUIRE_APPROVAL → approve → resume original ledger op` loop. Today
PR #2 S14 + S15 + S16 ship the approval state model + REST API + adapter resume
proto, but **nothing actually unblocks the original deferred reservation when
an approver clicks approve.** The row goes `pending → approved`, the SP
returns transitioned=true, and that's it — no idempotent ledger transaction
lands. So adapters waiting on `ResumeAfterApproval` never see real progress.

This is the S15 / S16 end-to-end closer.

## Files to read first

- `services/ledger/migrations/0026_approval_requests.sql` — schema
  (`requested_effect` JSONB carries the deferred operation)
- `services/ledger/migrations/0033_resolve_approval_ttl_atomic.sql` — current
  resolve_approval_request SP (atomic TTL guard from round 9)
- `services/ledger/migrations/0012_post_ledger_transaction.sql` and the
  follow-on SPs (0013 commit_estimated, 0014 provider_reported, 0015 release,
  0016 invoice_reconcile) — pattern for a real ledger-mutating SP
- `services/sidecar/src/server/adapter_uds.rs` — Resume handler stub
- `proto/spendguard/sidecar_adapter/v1/adapter.proto` — `ResumeAfterApproval` RPC
- `services/control_plane/src/main.rs:resolve_approval` — currently the only
  caller of resolve_approval_request

## Acceptance criteria

- New migration `services/ledger/migrations/0036_approval_bundling_sp.sql`
  with a SP `bundle_approved_decision_into_reservation(p_approval_id UUID)`
  that, atomically:
  1. Locks the approval row `FOR UPDATE`, asserts `state='approved'` and not
     already bundled
  2. Reads `requested_effect` JSONB; reconstructs the original
     `RequestDecisionRequest` shape (or whichever subset is needed)
  3. Calls into the existing `post_ledger_transaction` SP (or its
     commit/release variants depending on `requested_effect.kind`) with a
     **deterministic** `idempotency_key` that includes `approval_id` so the
     bundling itself is idempotent
  4. Marks the approval row `bundled_at = clock_timestamp()` and
     `bundled_ledger_transaction_id = <tx_id>` (add columns via the same
     migration; respect round-4 trigger immutability — terminal state stays
     terminal, only adds these never-mutate-after-set fields)
  5. Returns `(ledger_transaction_id, reservation_ids[])` for the API
- New ledger gRPC RPC `Ledger.PostBundledApprovalTransaction(approval_id) →
  (ledger_transaction_id, reservation_ids)`. Wire in
  `services/ledger/src/main.rs` + proto.
- `services/sidecar/src/server/adapter_uds.rs:ResumeAfterApproval` handler
  calls the new RPC. Replace stub with real path. Surface
  `ledger_transaction_id` + reservation IDs in the response.
- The S15 control_plane resolve_approval handler should optionally trigger
  bundling itself (defensive: if approver hits resolve and bundling SP is
  available, kick it off) — or document why the sidecar-driven path is
  authoritative.

## Pattern references

- Round-9 atomic TTL guard (migration 0033) shows how to add a new check
  inside `FOR UPDATE` without breaking existing invariants
- Round-4 immutability trigger (migration 0029) — your new `bundled_at` and
  `bundled_ledger_transaction_id` columns must be added to the trigger's
  "frozen-once-set on terminal" check
- Migration 0034 cross-row check pattern (decision_id LIKE 'ledger:%') for
  any new producer_id constraint on the bundling tx's audit row

## Verification

- Postgres smoke test: insert pending approval, resolve approved,
  call `bundle_approved_decision_into_reservation`, assert reservation
  row appears with the correct linked approval_id + idempotency_key
- New demo mode `make demo-up DEMO_MODE=approval` (or extend `agent`) that
  exercises `REQUIRE_APPROVAL → POST /v1/approvals/:id/resolve approved →
  ResumeAfterApproval → real reservation lands`
- Integration test in `services/ledger/tests/` covering double-resolve =
  exactly one bundled tx (idempotency)

## Commit + close

```
feat(s14/s15/s16): approval bundling SP closes resume loop (followup #9)

PR #2 shipped approval state + REST + resume proto, but no SP actually
unblocked the deferred ledger op when an approver clicked approve.
This adds bundle_approved_decision_into_reservation that atomically:
locks the approval, reads requested_effect, calls the matching
post_ledger_transaction variant with an approval-derived idempotency
key, marks the approval bundled. ResumeAfterApproval RPC now wires
to it.

Tests: postgres smoke + new demo approval mode.
```

After merge: `gh issue close 9 --comment "Shipped in <commit-sha>"`.
