# Drill: approval TTL wave

Quarterly drill. Validates the approval state machine + sweeper
under burst load: when many approvals expire simultaneously
(e.g. a tenant configured a short TTL contract rule and the
approver team is offline overnight), the TTL sweeper MUST
process the burst correctly without:

- Dropping or double-expiring rows.
- Bypassing the round-9 atomic TTL guard
  (`resolve_approval_request` SP: user-driven approve/deny on a
  TTL-expired pending row → 409 CONFLICT).
- Letting any approval get stuck in `pending` after TTL.
- Missing the round-5 system-actor injection (rows must end up
  with `resolved_by_subject = 'system:ttl-sweeper'` /
  `resolved_by_issuer = 'system:spendguard'`).

## What this drill exercises

- Migration 0030 (round 5) — TTL sweeper system-actor injection.
- Migration 0033 (round 9) — atomic TTL guard inside the
  resolve_approval_request SP.
- Migration 0035 (followup #4) — notification outbox writes for
  the `expired` transition.
- The S14 sweeper helper `expire_pending_approvals_due()` under
  load.

## Symptoms (what on-call sees)

- Alert A8 `SpendGuardApprovalLatencyHigh` (or its expired-tail
  variant) firing.
- `approval_requests` query: many rows with `state='pending' AND
  ttl_expires_at < now()`.
- `approval_events` query: burst of `to_state='expired'` events
  in a short window.
- `approval_notifications` query: corresponding burst of
  `transition_kind='expired'` rows pending dispatch.
- User-visible impact: adapters waiting on
  ResumeAfterApproval get the typed error indicating the
  approval lapsed → caller raises typed exception.

## First check

```bash
# 1. Snapshot pending approvals past TTL.
psql -h $LEDGER_PG_HOST -U spendguard -d spendguard_ledger -c "
  SELECT count(*) AS stuck_pending,
         min(ttl_expires_at) AS oldest_overdue
    FROM approval_requests
   WHERE state = 'pending'
     AND ttl_expires_at < now();
"
# Non-zero stuck count means sweeper isn't keeping up.

# 2. Recent expire-event rate.
psql -h $LEDGER_PG_HOST -U spendguard -d spendguard_ledger -c "
  SELECT count(*) AS expired_in_5min
    FROM approval_events
   WHERE to_state = 'expired'
     AND occurred_at > now() - interval '5 minutes';
"

# 3. Sweeper liveness — when did it last act?
psql -h $LEDGER_PG_HOST -U spendguard -d spendguard_ledger -c "
  SELECT max(occurred_at) AS last_sweep_action
    FROM approval_events
   WHERE actor_subject = 'system:ttl-sweeper';
"

# 4. Notification outbox backlog from the burst.
psql -h $LEDGER_PG_HOST -U spendguard -d spendguard_ledger -c "
  SELECT count(*) AS pending_dispatch
    FROM approval_notifications
   WHERE transition_kind = 'expired'
     AND pending_dispatch = TRUE;
"
```

## Mitigation (short-term unblock)

### Sweeper stalled (step 3 shows last action > 1 minute ago)

The TTL sweeper isn't running. Either:

1. **Pod down**: `kubectl get pods -l app.kubernetes.io/component=ttl-sweeper`
   and restart if not Running.
2. **Lease lost**: per the `lease-lost-mid-batch.md` drill —
   sweeper logs show "lease expired locally" warns. Same
   remediation: restart the pod.

### Sweeper running but burst too big

The sweeper batches via `expire_pending_approvals_due()` which
processes all overdue rows in one SP call. If the burst is huge
(thousands of rows), the SP can take a long time inside a single
transaction.

1. **Check sweeper logs** for the SP completion line:
   `expired N approvals via sweeper`.
2. **If the SP looks stuck**: check Postgres for
   long-running transactions:
   ```sql
   SELECT pid, now() - xact_start AS duration, query
     FROM pg_stat_activity
    WHERE state <> 'idle'
      AND xact_start IS NOT NULL
    ORDER BY xact_start;
   ```
   The sweeper holding the lock prevents new approvals from
   being created/resolved (they'd wait on the row-level lock).
3. **Operator-supervised batch limit** (S14-followup): a future
   migration would cap the SP's per-call batch size; for now
   the sweeper either completes or operators wait it out.

### Mass-expire was intended (e.g. test tenant)

No action required. Rows correctly expired with system actor +
notifications enqueued.

## Escalation

- **5 minutes** of growing stuck-pending count → page approver
  oncall (responsible for the contract that set short TTLs).
- **15 minutes** without sweeper progress → page platform oncall;
  TTL sweeper service may need restart.
- **30+ minutes** with adapters waiting → page engineering
  manager. Adapters' typed "approval lapsed" exceptions imply
  user impact.

## Rehearsal

```bash
# 1. Bring up demo with TTL=5s so we can create a burst quickly.
SIDECAR_TTL_SECONDS=5 make demo-up DEMO_MODE=ttl_sweep
# (The PR #6 ttl_sweep mode wires this end-to-end.)

# 2. Generate a burst of pending approvals via direct SQL
# (workaround: real adapter-driven approvals in burst would
# require a test contract with REQUIRE_APPROVAL rules).
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger -c "
  INSERT INTO approval_requests
    (approval_id, tenant_id, decision_id, audit_decision_event_id,
     state, ttl_expires_at, approver_policy, requested_effect,
     decision_context)
  SELECT
    gen_random_uuid(),
    '00000000-0000-4000-8000-000000000001',
    gen_random_uuid(),
    gen_random_uuid(),
    'pending',
    clock_timestamp() + interval '500 ms',
    '{}'::jsonb, '{}'::jsonb, '{}'::jsonb
  FROM generate_series(1, 50);
"

# 3. Wait past TTL.
sleep 2

# 4. Trigger the sweeper SP directly (it would normally be
# called by the ttl-sweeper service):
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger -c "
  SELECT expire_pending_approvals_due() AS expired_count;
"
# Expected: 50.

# 5. Verify all rows are now expired with system actor.
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger -c "
  SELECT state, resolved_by_subject, count(*)
    FROM approval_requests
   WHERE state IN ('expired', 'pending')
   GROUP BY state, resolved_by_subject;
"
# Expected: state=expired, resolved_by_subject=system:ttl-sweeper, count=50.

# 6. Verify notification rows landed (followup #4).
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger -c "
  SELECT transition_kind, count(*)
    FROM approval_notifications
   GROUP BY transition_kind;
"
# Expected: transition_kind=expired with N rows for the tenant
# IFF that tenant has a row in tenant_notification_config; 0
# rows if no config (followup #4 default behavior).

make demo-down
```

## Related

- L8 SLO definition: `docs/site/docs/operations/slos.md` row L8
- PR #2 round 5 commit `c084a26` — TTL sweeper SP fix (system
  actor injection)
- PR #2 round 9 commit `8810c14` — atomic TTL guard
- PR #14 commit `6f8d4d5` (followup #4) — notification outbox
  writes
- Sister drill: `lease-lost-mid-batch.md` covers the sweeper
  pod losing its lease (parent incident pattern)
