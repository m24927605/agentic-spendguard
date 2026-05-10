# Drill: audit chain forwarder backlog

Quarterly drill. Validates the audit chain end-to-end: when
`outbox-forwarder` is paused or slow, the `audit_outbox` queue
grows but no rows are lost; once the forwarder resumes, the queue
drains and `canonical_events` catches up.

This is the L4 SLO drill (audit-outbox-forward-lag p99 < 60s
within a 24h window).

## What this drill exercises

- The L4 SLO target.
- Alert A4 `SpendGuardOutboxLagHigh` firing at the documented
  threshold + clearing on recovery.
- The transactional invariant from S8: rows are NEVER dropped
  silently. They sit in `audit_outbox` with `pending_forward =
  TRUE` until either forwarded successfully (→
  `canonical_events`) or moved to `audit_signature_quarantine`
  (signature mismatch / unknown key).
- The forwarder's idempotency: re-processing the same backlog
  twice is a no-op against `canonical_events.audit_outbox_id`
  UNIQUE.

## Symptoms (what on-call sees)

- Alert A4 `SpendGuardOutboxLagHigh` paged on Prometheus.
- Dashboard panel `audit_outbox_pending_seconds` shows growing
  oldest-pending age.
- `SELECT count(*) FROM audit_outbox WHERE pending_forward = TRUE`
  → growing number, not draining.
- Dashboard's `canonical_events count` panel flat or growing
  slower than `audit_outbox` total count.
- User-visible impact: NONE for the producer side (sidecar /
  ledger / webhook continue to write audit rows). Audit-consuming
  flows (downstream BI, compliance exports) see staleness.

## First check

```bash
# 1. Confirm the forwarder pod is still running.
kubectl get pods -l app.kubernetes.io/component=outbox-forwarder -o wide
# All Running? Continue. Any CrashLoopBackOff? Tail logs:
kubectl logs <forwarder-pod> --tail=200

# 2. Pending count + oldest age (single SQL query):
psql -h $LEDGER_PG_HOST -U spendguard -d spendguard_ledger -c "
  SELECT count(*) AS pending,
         max(now() - recorded_at) AS oldest_age,
         min(recorded_at) AS oldest_recorded_at
    FROM audit_outbox
   WHERE pending_forward = TRUE;
"

# 3. Are forwards still landing? Compare canonical_events count
# now vs 1 minute ago:
psql -h $CANONICAL_PG_HOST -U spendguard -d spendguard_canonical -c "
  SELECT count(*) FROM canonical_events
   WHERE recorded_at > now() - interval '1 minute';
"
# 0 = forwarder is stalled. Non-zero = forwarder is processing,
# just behind on backlog.

# 4. Check the forwarder's lease state (validates "lease lost
# mid-batch" isn't actually the parent incident):
psql -h $LEDGER_PG_HOST -U spendguard -d spendguard_ledger -c "
  SELECT holder_workload_id, expires_at, expires_at < now() AS expired
    FROM coordination_leases
   WHERE lease_name = 'outbox-forwarder';
"
```

If step 4 shows `expired = TRUE` AND no holder, the parent incident
is "lease lost mid-batch" — see that drill instead.

## Mitigation (short-term unblock)

Route depends on what step 3 shows:

### Forwarder stalled (step 3 returns 0)

Options in order of escalation:

1. **Restart the forwarder pod**:
   ```bash
   kubectl delete pod <forwarder-pod>
   ```
   New pod acquires lease via leader election + resumes from
   `audit_outbox.pending_forward = TRUE` rows ordered by
   `recorded_at`. The forward loop is idempotent so partial
   replays are safe.
2. **If multiple replicas and only one is stuck**: that pod is
   in a bad state — kill it; standby takes over.
3. **If all replicas show the same stall**: the canonical_ingest
   side is rejecting → check `audit_signature_quarantine` for
   recent rows. The "strict-signature-quarantine-spike" drill
   covers that scenario.

### Forwarder processing but backed up (step 3 non-zero)

1. **Increase forwarder replica count temporarily** (operator
   ack required):
   ```bash
   kubectl scale deployment outbox-forwarder --replicas=2
   ```
   Note: only the leader processes work — extra replicas just
   make takeover faster. To actually parallelise, the workload
   must be partitioned (per-tenant scope_id, not a single global
   lease). This is a known scope limit.
2. **Tune `OUTBOX_FORWARDER_BATCH_SIZE`** (env var; default 100):
   higher values trade per-batch latency for throughput. Bump
   to 500–1000 during recovery, restore default after lag
   clears.

## Escalation

- **15 minutes** of growing pending count → page platform oncall.
- **60 minutes** with no recovery → page engineering manager;
  consider a manual SQL drain (very risky — only if you
  understand the dedup invariants).
- **24 hours** lag → SLO violation; document in
  `docs/site/docs/operations/drill-log.md`.

## Rehearsal

```bash
# 1. Bring up demo with full chain.
make demo-up DEMO_MODE=invoice

# 2. Pause the forwarder.
docker pause spendguard-outbox-forwarder

# 3. Generate audit traffic by re-running the demo a few times.
for i in 1 2 3; do
  make demo-up DEMO_MODE=decision
done

# 4. Confirm pending count grows.
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger \
  -c "SELECT count(*) AS pending FROM audit_outbox WHERE pending_forward = TRUE;"
# Expected: > 5 rows pending.

# 5. Resume the forwarder.
docker unpause spendguard-outbox-forwarder
sleep 10

# 6. Confirm drain.
docker exec spendguard-postgres psql -U spendguard -d spendguard_ledger \
  -c "SELECT count(*) AS pending FROM audit_outbox WHERE pending_forward = TRUE;"
# Expected: pending count back to baseline (0 or close).

# 7. Confirm canonical_events caught up.
docker exec spendguard-postgres psql -U spendguard -d spendguard_canonical \
  -c "SELECT count(*) FROM canonical_events;"
# Expected: count matches the total audit rows generated.

make demo-down
```

## Related

- L4 SLO definition: `docs/site/docs/operations/slos.md` row L4
- Alert: A4 `SpendGuardOutboxLagHigh` in
  `deploy/observability/prometheus-rules.yaml`
- Sister drill: `lease-lost-mid-batch.md` — when the forwarder
  stops because its lease went stale, not because it's slow
