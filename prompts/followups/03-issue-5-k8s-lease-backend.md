# Followup #5 — K8sLease backend (S5-followup)

GitHub issue: https://github.com/m24927605/agentic-flow-cost-evaluation/issues/5

## Goal

Replace the `K8sLease` stub with a real `kube`-rs integration backed by
`coordination.k8s.io/Lease` resource. Today it returns
`LeaseError::ModeUnavailable` for every call; the Helm chart has a fail-gate
(PR #2 round 5, commit `c084a26`) blocking `leaderElection.mode=k8s` until
this followup lands.

## Files to read first

- `services/leases/src/lib.rs` — full file:
  - `LeaseManager` trait (~line 67)
  - `PostgresLease` impl (reference) — atomic CAS via SP
  - `K8sLease` stub (~line 240)
  - `LeaseState` enum + `is_leader_now()` (round 9)
- `services/ttl_sweeper/src/main.rs` — consumer of the lease, uses
  `is_leader_now()` (round 9) to gate work
- `services/outbox_forwarder/src/main.rs` — same consumer pattern
- `charts/spendguard/templates/ttl-sweeper.yaml` + `outbox-forwarder.yaml` —
  current fail-gate at top of the templates blocks `mode=k8s`

## Acceptance criteria

- `kube = "0.95"` and `k8s-openapi = { features = ["latest"] }` deps added
  to `services/leases/Cargo.toml`
- `K8sLease::try_acquire` performs the canonical Lease leader-election dance:
  1. GET `coordination.k8s.io/v1/namespaces/{ns}/leases/{lease_name}`
  2. If absent → CREATE with `holderIdentity = workload_id`,
     `leaseDurationSeconds`, `acquireTime = now()`, `renewTime = now()`,
     `leaseTransitions = 1`. Return `Granted::Leader { token, expires_at, transition_count }`.
  3. If present and `holderIdentity == workload_id` → PATCH renewTime,
     return `Leader { token, expires_at, transition_count }` (transition_count
     stays the same).
  4. If present and another holder, but `renewTime + leaseDurationSeconds <
     now()` (expired) → PATCH with our identity + renewTime, increment
     `leaseTransitions`. Return `Leader { ..., transition_count: prev+1 }`.
  5. Else → return `Standby { holder_workload_id, observed_expiry }`.
- `K8sLease::release`: PATCH `holderIdentity = null`, `renewTime = null`
  (best-effort; other pods will take over via expiry anyway).
- `LeaseState::Leader::token` for k8s backend: derive from
  `(uid, leaseTransitions)` so it's stable per leader epoch and unique
  across pods. PostgresLease uses a UUID; k8s needs an analogous identity.
- Helm chart adds RBAC: new ClusterRole + RoleBinding with verbs
  `leases.coordination.k8s.io: [get, list, watch, create, update, patch, delete]`
  scoped to the workload's namespace. Bind to ttl-sweeper + outbox-forwarder
  ServiceAccounts.
- Helm fail-gate for `mode=k8s` in `ttl-sweeper.yaml` + `outbox-forwarder.yaml`
  removed (or downgraded to require explicit `acknowledgeK8sLease=true`
  during the soak window).
- Update `docs/site/docs/operations/multi-pod.md` runbook with k8s vs
  postgres mode tradeoffs + diagnostic commands.
- 5+ unit tests using `kube`'s mock or kind-based integration test:
  - acquire-when-absent
  - renew-while-holder
  - takeover-after-expiry
  - standby-while-other-holds-and-fresh
  - release-best-effort

## Pattern references

- `PostgresLease` is the gold standard for the trait contract. Match its
  semantics exactly: `LeaseAttempt { state, event_type }` where `event_type`
  is one of "acquired", "renewed", "transitioned", "denied", "released".
- Round-9 `is_leader_now()` (services/leases/src/lib.rs) — the new k8s
  Leader variant must populate `expires_at` from `renewTime + leaseDurationSeconds`
  so callers' `is_leader_now()` correctly returns false when the local
  cached state is stale.

## Verification

- `cargo test --lib` in services/leases passes including new tests
- `kind create cluster` + helm install with `leaderElection.mode=k8s`,
  bring up 2 ttl-sweeper replicas, kill the leader pod, confirm the
  standby takes over within `leaseDurationSeconds`, no double-sweeps
  in the audit log
- Existing `mode=postgres` and `mode=disabled` paths unchanged

## Commit + close

```
feat(s5): K8sLease backend over coordination.k8s.io/Lease (followup #5)

Replaces the ModeUnavailable stub with a real kube-rs integration.
ttl-sweeper + outbox-forwarder can now run multi-pod with k8s leader
election; postgres backend stays as the default for sites without
the kube ServiceAccount RBAC.

Helm chart adds the lease RBAC ClusterRole + binding. The
chart-template fail-gate for mode=k8s is removed.

Tests: 5 unit tests + kind cluster e2e (2 replicas, kill-leader,
no double-sweep observed).
```

After merge: `gh issue close 5 --comment "Shipped in <commit-sha>"`.
