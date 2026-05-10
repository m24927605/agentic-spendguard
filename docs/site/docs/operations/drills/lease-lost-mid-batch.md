# Drill: lease lost mid-batch

Quarterly drill. Validates round-9's expiry-aware `is_leader_now()`
gating in `outbox-forwarder` and `ttl-sweeper`: if a worker's local
lease state goes stale (renewal task stalls past `expires_at`), the
worker MUST stop processing on its next loop iteration, not keep
working off the cached `Leader` value.

This drill is the live counterpart to the unit tests in
`services/leases/src/lib.rs::tests::is_leader_now_*`.

## What this drill exercises

- `services/leases/src/lib.rs::LeaseState::is_leader_now()` — the
  expiry-aware leader check added in PR #2 round 9 (commit
  `8810c14`).
- `services/outbox_forwarder/src/main.rs` and
  `services/ttl_sweeper/src/main.rs` — the consumer-side gate that
  calls `is_leader_now()` instead of pattern-matching the variant.
- The worker's `warn!(expires_at = %expires_at, ...)` log line that
  fires when the cached state is stale.

## Symptoms (what on-call sees)

When the renewal task stalls (e.g. Postgres lease backend hits a
slow replica), the worker pod does NOT crash. Instead:

- Worker log shows the warn line: `lease expired locally; skip
  batch until renewed`.
- `audit_outbox.pending_forward = TRUE` count starts climbing on
  the affected tenant (only if outbox-forwarder is the affected
  worker).
- `reservations.current_state = 'reserved' AND ttl_expires_at <
  now()` count starts climbing (only if ttl-sweeper is the affected
  worker).
- A4 (`SpendGuardOutboxLagHigh`) or its ttl-sweeper counterpart
  may eventually fire if the stall is long enough.

## First check

```bash
# Identify the worker pod and check its log for the local-expire warn:
kubectl logs -l app.kubernetes.io/component=outbox-forwarder --tail=200 \
  | grep -F "lease expired locally"

# Confirm the lease row in postgres:
psql -h $LEDGER_PG_HOST -U $LEDGER_PG_USER -d spendguard_ledger -c "
  SELECT lease_name, holder_workload_id, expires_at, expires_at < clock_timestamp() AS already_expired
    FROM coordination_leases
   WHERE lease_name IN ('outbox-forwarder', 'ttl-sweeper');
"

# If `already_expired = TRUE` AND another worker hasn't taken over,
# the renewal path is broken (not just stalled).
```

## Mitigation (short-term unblock)

If a worker is stuck in the warn loop and not making progress:

1. **Check Postgres connectivity** from the worker pod:
   ```bash
   kubectl exec <worker-pod> -- pg_isready -h $LEDGER_PG_HOST -U spendguard
   ```
   If unreachable → escalate to platform/oncall (Postgres outage
   is the parent incident).
2. **Restart the affected worker pod** to force a fresh
   `try_acquire` cycle:
   ```bash
   kubectl delete pod <worker-pod>
   ```
   Standby replicas (or the same Deployment's replacement) take
   over within `leaderElection.ttlMs`.
3. **Verify takeover** in the postgres lease row above:
   `holder_workload_id` should change to the new pod's id +
   `expires_at` should advance.

## Escalation

- **5 minutes** sustained: page outbox-forwarder / ttl-sweeper
  team primary (per
  `docs/site/docs/operations/slos.md` owner table).
- **15 minutes** sustained without takeover: page platform
  oncall — implies the Postgres lease backend itself is broken,
  not just one worker pod.
- **30+ minutes**: escalate to engineering manager + start
  considering manual SQL release of the lease (carefully — risk
  of double-leadership).

## Rehearsal (compose-based demo)

Validate this drill against the local demo cluster without
touching prod:

```bash
# 1. Bring up the demo with both workers running.
make demo-up DEMO_MODE=invoice
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger \
  -c "SELECT lease_name, holder_workload_id, expires_at FROM coordination_leases ORDER BY lease_name;"

# 2. Simulate renewal stall: pause the worker so its renewer can't
# run. The local lease state stays `Leader` but expires_at goes
# stale.
docker pause spendguard-outbox-forwarder

# 3. Wait past leaderElection.ttlMs (compose default: ~10s).
sleep 15

# 4. Unpause. The next poll iteration should hit is_leader_now() =
# false and emit the warn line BEFORE attempting forward_batch.
docker unpause spendguard-outbox-forwarder
sleep 3
docker logs spendguard-outbox-forwarder 2>&1 | tail -20 \
  | grep -E "lease expired locally|lease state = LEADER"

# Expected output: at least one "lease expired locally" line BEFORE
# the next "lease state = LEADER" (renewed).

# 5. Cleanup.
make demo-down
```

Run this rehearsal once a quarter; rotate operators so each on-call
person has executed it at least once before being primary.

## Related

- `docs/site/docs/operations/slos.md` — D2 (stale fencing lease)
  covers the sidecar-side analog where a fencing-scope lease ages
  out and a new sidecar pod takes over with `fencing_epoch = N+1`.
- PR #2 round 9 commit `8810c14` — the actual `is_leader_now()`
  implementation.
