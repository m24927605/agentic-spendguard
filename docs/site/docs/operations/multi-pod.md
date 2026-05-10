# Multi-pod deployment runbook (Phase 5 S5)

This runbook covers safe horizontal / multi-node deployment of the
sidecar, outbox-forwarder, and ttl-sweeper services after S1 (lease
primitive), S2 (per-pod producer instance id), S3 (Ledger fencing
RPC), and S4 (sidecar fencing-lease lifecycle) have shipped.

**TL;DR**: outbox-forwarder and ttl-sweeper are safe to scale to
N pods with leader election. Sidecar is a DaemonSet — one pod per
node — and only one sidecar pod at a time may hold the configured
fencing scope; the rest fail-closed at startup waiting for takeover.
This is **active/standby**, not horizontal scaling.

## Per-component scaling model

### outbox-forwarder

- **Type**: Deployment.
- **Multi-pod model**: leader election. Only the leader polls the
  `audit_outbox` table and forwards to canonical-ingest; standby
  replicas heartbeat the lease and are eligible to take over.
- **Helm gate**: `outboxForwarder.replicas > 1` is rejected when
  `leaderElection.mode = disabled` (S1 gate; works as designed).
- **What to set**:
  ```yaml
  outboxForwarder:
    replicas: 2  # or 3 for region-spread
  leaderElection:
    mode: postgres   # or k8s after S5+S7
    region: us-west-2
    ttlMs: 15000
    renewIntervalMs: 5000
  ```
- **Failover behavior**: when the active leader pod dies, the
  Postgres lease's TTL expires after `ttlMs` (15s default). A
  standby calls `acquire_lease` SP and wins; forwarding resumes.
  No duplicate canonical events because `audit_outbox.pending_forward`
  is the durable cursor.

### ttl-sweeper

- **Type**: Deployment.
- **Multi-pod model**: identical to outbox-forwarder (leader
  election; only the leader polls + releases expired reservations).
- **Helm gate**: `ttlSweeper.replicas > 1` rejected with
  `leaderElection.mode=disabled` (S1).
- **Recommended**: `replicas: 2` for HA. Higher counts add no
  throughput (only one pod sweeps).

### sidecar

- **Type**: DaemonSet (one pod per node by design — co-located
  with workload pods that mount the UDS adapter socket).
- **Multi-pod model**: each pod has a unique `workload_instance_id`
  derived from `metadata.name` via the downward API (S2). At
  startup each pod calls `Ledger.AcquireFencingLease` (S4); the
  Ledger SP serializes via `FOR UPDATE` and grants the lease to
  exactly one pod. The other pods fail-closed at startup with
  `S4: acquire fencing lease at startup` and stay in
  CrashLoopBackOff or Pending.
- **This is NOT horizontal scaling**. There is one active
  decision-serving sidecar per fencing scope at any moment.
- **Why DaemonSet then?** Co-location: each node has a UDS
  socket reachable from app pods on the same node. The fencing
  scope is per-tenant (or per-tenant×region); only one node's
  sidecar holds it.
- **Helm gate**: `sidecar.acknowledgeMultiPod=true` is required
  to express explicit operator awareness of the active/standby
  semantics. `workloadInstanceIdOverride` must NOT be set when
  multi-pod is enabled (override means single-pod identity).

## Failover and takeover

### Sidecar fencing takeover

When the active sidecar pod dies (OOM, eviction, node failure):

1. The pod's `AcquireFencingLease` lease times out after
   `SPENDGUARD_SIDECAR_FENCING_TTL_SECONDS` (default 30s).
2. Standby sidecars on other nodes that crashed at startup are
   restarted by the kubelet. On restart they call
   `Ledger.AcquireFencingLease` again.
3. The Ledger SP sees the previous lease expired and grants the
   new pod a `takeover` action with `epoch_increment = 1`. The
   new pod's audit rows now sign with `fencing_epoch = N+1`.
4. Any in-flight decisions from the old pod that try to commit
   with `fencing_epoch = N` get rejected by the Ledger's CAS
   check (`FENCING_EPOCH_STALE` error). The audit invariant
   ("no effect without valid epoch") holds.

Operator dashboard surfaces:
- `spendguard_sidecar_fencing_epoch` gauge (per pod)
- `spendguard_sidecar_fencing_acquire_action_total{action}`
  counter (acquire / renew / takeover) — a takeover spike means
  failover happened.

### Outbox-forwarder leader change

- `coordination_lease_history` table is the audit log: every
  takeover writes a row with `event_type = 'taken_over'` and
  `transition_count + 1`.
- Operator monitors `spendguard_outbox_forwarder_leader_age_seconds`
  histogram and `coordination_lease_history` rows.

## Rollback to single-pod

For all three services, rollback is just:

```yaml
sidecar:
  acknowledgeMultiPod: false  # if you set it
outboxForwarder:
  replicas: 1
ttlSweeper:
  replicas: 1
```

No DB surgery is needed. The lease/fencing state is in Postgres
and is renewed/taken-over by whichever pod is alive.

## Chaos drill checklist

The S5 acceptance criteria call for a "kind test: two sidecars,
two forwarders, two sweepers, all healthy." Until that automated
test lands (deferred to S5-followup), operators should manually
verify:

1. Deploy with `outboxForwarder.replicas=2`,
   `ttlSweeper.replicas=2`, sidecar DaemonSet on 2-node cluster.
2. Verify `coordination_leases` shows exactly one leader per
   `lease_name` (`outbox-forwarder`, `ttl-sweeper`).
3. Verify `fencing_scopes` shows exactly one
   `current_holder_instance_id` per scope.
4. `kubectl delete pod <leader>`. Wait `ttlMs + grace` (default
   ~30s).
5. Verify `coordination_lease_history` has a new
   `taken_over` row.
6. Verify ledger / canonical-ingest see no duplicate audit rows
   (`audit_outbox_global_keys` UNIQUE on
   `(tenant, workload_instance_id, producer_sequence)` rejects
   duplicates).
7. Repeat for sidecar: `kubectl delete pod <active-sidecar>` —
   the standby sidecar on another node takes over with
   epoch+1.

## Observability invariants

Every S1+S4-aware deployment should alert on:

- `coordination_lease_history` rows with `event_type='taken_over'`
  more than 1× per hour per lease — likely lease-flap (TTL too
  short or network partition).
- `fencing_scope_events` with `action='promote'` more than 1× per
  hour — sidecar takeover storm.
- Sidecar pods in `CrashLoopBackOff` with
  `acquire fencing lease at startup` in their logs for more than
  5 minutes — usually means the seeded scope row is missing or
  the workload identity collides.

## Known limitations (S5-followup)

1. **Per-pod fencing scope** is not yet supported. All sidecar
   pods on all nodes share the configured `sidecar.fencingScopeId`.
   True horizontal scaling requires per-pod scope assignment;
   tracked as a S5-followup.
2. **Automated kind test** for the chaos drill above is deferred.
3. **Sidecar pre-stop drain** during takeover is in place (S4)
   but the takeover SP doesn't yet revoke the prior holder's
   lease — it just lets the TTL expire. Faster takeovers will
   need an explicit revoke RPC.
