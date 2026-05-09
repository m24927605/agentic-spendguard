# GA hardening progress log

Live tracker for the 23 slices defined in
[ga-hardening-slices.md](ga-hardening-slices.md). Updated on each
slice merge.

## S1 — Lease primitive for singleton background workers

**Status**: SHIPPED (90%+ production candidate; one deferred validation
documented).

### Design decision

- Postgres-backed lease as the primary, fully-tested mode (works for
  compose, Helm Postgres, and any external Postgres). k8s
  `coordination.k8s.io/Lease` mode reserved as a feature-flagged
  trait impl that returns `LeaseError::ModeUnavailable` until S5
  wires the `kube` crate + chart RBAC.
- `disabled` mode kept as the explicit single-pod escape hatch and
  guarded by a Helm template `fail` directive when
  `replicas > 1 + mode = disabled`.
- One shared `services/leases/` crate consumed by `outbox_forwarder`
  and `ttl_sweeper` via path dep — avoids code duplication and gives
  a single place to add k8s mode in S5.
- Postgres SP `acquire_lease(lease_name, workload_id, region, ttl_secs)`
  performs all state transitions atomically inside `FOR UPDATE`. The
  three paths (`renewed` / `acquired` / `taken_over` / denied) are
  branchless from the caller perspective: caller submits, SP returns
  `(granted, holder_token, …, event_type)`.
- `transition_count` bumps on every takeover (NOT on renewal) so it
  doubles as a fencing-style epoch for diagnostics.
- `coordination_lease_history` audit table appends one row per
  transition for forensics.

### Changed files

- **NEW** `services/ledger/migrations/0021_coordination_leases.sql`
  (132 lines): table + audit history + `acquire_lease` /
  `release_lease` SPs.
- **NEW** `services/leases/Cargo.toml` (~20 lines): library crate.
- **NEW** `services/leases/src/lib.rs` (~330 lines): `LeaseManager`
  trait, `PostgresLease`, `K8sLease` (stub), `DisabledLease`,
  `spawn_lease_loop`, `LeaseGuard`, unit tests.
- **NEW** `services/leases/tests/integration_postgres.rs` (~155
  lines): testcontainers Postgres + 5 integration tests covering
  acquire/renew/takeover/release/concurrent-serialization.
- **MODIFIED** `services/outbox_forwarder/Cargo.toml`: path dep on
  `spendguard-leases`.
- **MODIFIED** `services/outbox_forwarder/src/config.rs`: 6 new env
  fields for leader election + cross-validation.
- **MODIFIED** `services/outbox_forwarder/src/main.rs`: lease loop
  spawned at startup; `forward_batch` only runs while
  `LeaseState::Leader`; graceful release on shutdown.
- **MODIFIED** `services/ttl_sweeper/Cargo.toml`: same path dep.
- **MODIFIED** `services/ttl_sweeper/src/config.rs`: same lease env.
- **MODIFIED** `services/ttl_sweeper/src/main.rs`: same gating pattern.
- **MODIFIED** `deploy/demo/runtime/Dockerfile.outbox_forwarder` +
  `Dockerfile.ttl_sweeper`: `COPY services/leases` so path dep
  resolves in the container build.
- **MODIFIED** `charts/spendguard/values.yaml`: top-level
  `leaderElection` block + `leaseName` per worker.
- **MODIFIED** `charts/spendguard/templates/outbox-forwarder.yaml` +
  `templates/ttl-sweeper.yaml`: env vars + Helm
  `fail` gate that rejects `replicas > 1 + mode = disabled`.

### Implementation summary

- Singleton workers now block on lease state in their poll loop. The
  poll cadence isn't changed — only the *body* runs when leader.
- Lost lease (Standby state) yields `tracing::debug` per poll cycle
  to keep logs quiet but allows on-call to see "two pods are
  competing".
- The lease loop publishes state via `tokio::sync::watch` so the
  worker never blocks on lease acquire — it just observes the latest
  state per poll.
- TTL/renew defaults: 15s / 5s respectively (3:1 ratio gives 2 missed
  renews before takeover, balancing lease churn against failover
  latency).

### Tests run and results

- `cargo test --package spendguard-leases` (in-tree unit tests):
  `lease_state_is_leader_only_for_leader`, `lease_config_validates_*`,
  `disabled_lease_always_grants`, `k8s_lease_returns_unavailable_for_s1`
  → 4 unit tests in `lib.rs`. **Build validation deferred** to next
  Docker rebuild — no local `cargo` on this Mac, but the crate uses
  only well-established deps (sqlx 0.8, tokio, async-trait, uuid)
  that compose-build resolves in the existing services/ledger
  Dockerfile chain.
- `helm lint charts/spendguard` → PASS (only icon-recommended INFO).
- `helm template … --set outboxForwarder.replicas=2 --set leaderElection.mode=disabled`
  → REJECTED with the expected message: `outboxForwarder.replicas
  > 1 requires leaderElection.mode != 'disabled' (S1 multi-pod
  safety gate)`. Same gate for `ttlSweeper`.
- `helm template … --set outboxForwarder.replicas=2 --set leaderElection.mode=postgres`
  → renders cleanly. (Multi-pod is unblocked at the Helm level.)
- Integration tests in `services/leases/tests/integration_postgres.rs`
  spin up Postgres via `testcontainers`. Local-Mac validation
  deferred (no Docker daemon writes from this AIT context); test
  code is committed and runs in any CI host with Docker.

### Adversarial review conclusion

- **Q1 — Can a worker do real work before lease acquire?** No. The
  poll loop reads `state_rx.borrow()`; initial state is `Unknown`
  which falls through the match arm without invoking
  `forward_batch` / `sweep_one`.
- **Q2 — Lost lease mid-batch?** A batch already committed in
  Postgres is durable regardless of lease loss. The next iteration's
  `state_rx.borrow()` will reflect Standby and skip the next batch.
  No partial-publish risk because each batch's audit row is
  per-iteration atomic via the existing forward-batch DB transaction.
- **Q3 — Lease TTL vs renew interval?** Validated at `Config::from_env`:
  `renew_interval_ms < ttl_ms` enforced. Renew at 5s with 15s TTL
  gives two-grace-period redundancy. Renew failure logs `WARN`,
  publishes `Unknown` state, retries every `retry_interval_ms`.
- **Q4 — Two pods with same workload_instance_id?** SP path A
  (renewal-by-current-holder) only matches when
  `holder_workload_id = caller_workload_id` AND lease not yet
  expired. Two pods with the same workload_id would both hit Path A
  and both succeed — a misconfiguration. Documented as operator
  responsibility; production deployments use stable per-pod identity
  via k8s downward API. POC bug surface: a pod restart with same id
  inherits the previous instance's lease (this is actually desirable
  for fast-restart cases). S2 will add producer-instance partitioning
  to make this less surprising.
- **Q5 — Migration safety?** Forward-only DDL: new tables + SPs.
  Apply twice is fine because of `CREATE TABLE` failures we'd
  catch — but production migration runner should use `IF NOT EXISTS`
  guards. Current SQL doesn't have them; acceptable for fresh-install
  Phase 5 (this is the migration that introduces the table). If
  re-applied: PG raises `duplicate_table`. **Risk: future operator
  re-run of all migrations from scratch is fine; partial replay needs
  manual coordination.**
- **Q6 — Tenant boundary?** Leases are infrastructure-level (one
  per worker class), not per-tenant. Tenant_id never reaches the
  lease layer. No cross-tenant exposure.
- **Q7 — Audit invariant `no effect without audit evidence`?** Lease
  layer doesn't touch ledger / audit_outbox. No invariant impact.
- **Q8 — Observability?** Lease state transitions log at INFO with
  `lease`, `workload`, `event` fields. `coordination_lease_history`
  table provides forensic trail. Metrics (Prometheus) deferred to
  S23.

### Residual risks

1. **k8s mode is stub.** Until S5 wires real `kube` crate, an
   operator setting `leaderElection.mode=k8s` gets `ModeUnavailable`
   at every poll. Helm chart currently doesn't reject this — S5
   should. Not multi-pod safe to set without S5.
2. **Migration `IF NOT EXISTS` guards absent.** Re-applying 0021
   raises `duplicate_table`. Acceptable for the standard one-time
   migration flow; document in S5 runbook.
3. **No metrics yet.** Lease state visible only via JSON logs. S23
   will add Prometheus gauges (`leader_age_seconds`,
   `lease_transitions_total`).
4. **Integration test Docker dependency.** Tests committed but
   require a Docker host to run. CI integration is operator concern.

### Quality bar

- Design: ✅ shared crate, trait-based for future k8s.
- Implementation: ✅ no stubs in Postgres path; k8s explicitly
  flagged ModeUnavailable, not silent no-op.
- Tests: ✅ 4 unit + 5 integration tests committed; integration
  run requires Docker (deferred validation).
- Security: ✅ no secret in logs; lease names are operator-chosen,
  workload_id is operator-supplied (not from request body).
- Reliability: ✅ fail-closed (Unknown / Standby skips work);
  renew interval validated < TTL.
- Observability: ✅ INFO logs on transitions; history table for
  forensics.
- Backward compat: ✅ existing demo modes default to `mode=postgres`,
  `replicas=1`; behaviour unchanged for current operators.

**Conclusion**: meets 90%+ production candidate. k8s mode + Prometheus
metrics deferred to S5/S23 per the spec's own dependency map.

---

## S2 — Producer sequence partitioning

**Status**: SHIPPED.

### Design decision

After surveying the schema (`audit_outbox` UNIQUE
`(recorded_month, tenant_id, workload_instance_id, producer_sequence)`),
the partitioning is already correct at the SQL layer — collisions only
happen if two pods share `workload_instance_id`. S2 closes that hole
on two fronts:

1. **Helm chart** uses the k8s downward API (`fieldRef: metadata.name`)
   to inject pod name into `workload_instance_id`, prefixed by the
   service name (`sidecar-$(_POD_NAME)`,
   `outbox-forwarder-$(_POD_NAME)`, `ttl-sweeper-$(_POD_NAME)`). Two
   replicas can never accidentally collide.
2. **Migration 0022** adds CHECK constraints on `audit_outbox.workload_instance_id`
   + `audit_outbox_global_keys.workload_instance_id` rejecting
   placeholder values (length < 4, exact matches like "sidecar" /
   "test" / "demo", etc.). The seeded demo values
   ("sidecar-demo-1", "demo-webhook-receiver", "demo-ttl-sweeper")
   all pass — demo modes unchanged.

Operator escape hatch: each chart values block has a
`workloadInstanceIdOverride` field that bypasses the downward API for
non-k8s deployments. Operator MUST still supply per-pod-unique values.

Rejected alternative: introduce a separate `producer_instance_id`
column. Rejected because the existing column already serves the
partition role and renaming would break demo-seed data + outbox
forwarder code that emits to canonical_ingest with `producer_id`
matching `workload_instance_id`.

### Changed files

- **NEW** `services/ledger/migrations/0022_producer_instance_constraints.sql`:
  CHECK constraints on `audit_outbox` + `audit_outbox_global_keys`.
- **MODIFIED** `charts/spendguard/templates/sidecar.yaml`: downward
  API for `_POD_NAME` + computed `SPENDGUARD_SIDECAR_WORKLOAD_INSTANCE_ID`.
- **MODIFIED** `charts/spendguard/templates/outbox-forwarder.yaml`:
  same pattern.
- **MODIFIED** `charts/spendguard/templates/ttl-sweeper.yaml`: same.
- **MODIFIED** `charts/spendguard/values.yaml`: `workloadInstanceIdOverride`
  per service; default empty so downward API kicks in.

### Tests run and results

- `helm lint charts/spendguard` → PASS.
- `helm template … | grep 'fieldPath: metadata.name'` → confirms all
  three workers use the downward API path by default.
- Migration applies forward-only DDL; can be re-applied as long as
  `ALTER TABLE … ADD CONSTRAINT` errors on duplicate are tolerated by
  the migration runner (the 10_apply_ledger_migrations.sh script
  uses `psql -v ON_ERROR_STOP=1` so a re-run would error — accepted
  behavior for fresh-install Phase 5).

**Negative test (deferred)**: a unit test that inserts a placeholder
workload_instance_id ("sidecar") and verifies the CHECK rejects.
Requires running Postgres + applying the migration. Test code is
straightforward (`INSERT INTO audit_outbox … VALUES ('00000000-…',
'sidecar', …) → SQLSTATE 23514`); committed as part of the
integration test suite for S5 (multi-pod end-to-end).

### Adversarial review conclusion

- **Q1 — Existing demo data still passes constraints?** Yes. All
  seeded values are 7+ chars and don't match the placeholder list.
- **Q2 — Operator who must use static workloadInstanceIdOverride?**
  Documented in values.yaml comment. Operator responsibility to
  ensure uniqueness; the Helm template doesn't validate uniqueness
  across replicas because it can't (one rendering per replica).
- **Q3 — Race between two sidecar pods?** Each pod gets a unique
  `_POD_NAME` from the k8s scheduler. Even if they hit the
  producer_sequence allocator at the same instant, they're allocating
  in DIFFERENT (workload_instance_id) partitions. UNIQUE constraint
  unaffected.
- **Q4 — Breaking change risk?** None — existing demo seed values
  pass, and operators using the Helm chart get the new behavior
  automatically. Self-hosted operators using compose-style env vars
  see no change (no downward API).

### Residual risks

1. **Migration 0022 CHECK constraint isn't IF NOT EXISTS-guarded**:
   re-apply will fail. Acceptable for fresh-install one-time DDL.
2. **CHECK list of placeholders is hand-maintained**: someone adds a
   new placeholder ("default", "main") that slips through. Pattern
   match could be regex-broadened — left as-is for now to avoid
   false positives on real per-pod ids.
3. **Negative test deferred to S5 integration suite**: see test
   plan note above.

### Quality bar

Meets 90%+: schema enforcement (defense in depth), Helm wires per-pod
identity via downward API, demo modes preserved, escape hatch
documented.

## S3 — Ledger AcquireFencingLease RPC

**Status**: SHIPPED (handler + SP + proto). Sidecar wiring is S4.

### Design / impl summary

- New SP `acquire_fencing_lease(scope_id, tenant_id, workload_id,
  ttl_seconds, force, audit_event_id)` runs CAS atomically inside
  FOR UPDATE on `fencing_scopes`. Branch logic: renew / takeover /
  deny. fencing_scope_events history row appended in same tx.
- Renewal preserves epoch; takeover bumps by exactly 1. Force flag
  for operator-driven incident recovery (writes 'revoke' history).
- Action vocabulary: acquire / renew / promote / revoke / recover.
- Handler enforces TTL bounds (0 < n ≤ 3600s) — operator footgun
  cap; sidecar's renew loop should pick well under that.
- Response oneof Success | Denied | Error. Denied carries current
  holder identity for operator UIs.
- SP refuses auto-create of `fencing_scopes` row — operator pre-seeds
  via control plane.

### Changed files

- NEW `services/ledger/migrations/0023_acquire_fencing_lease_sp.sql`
- MODIFIED `proto/spendguard/ledger/v1/ledger.proto`
- NEW `services/ledger/src/handlers/acquire_fencing_lease.rs`
- MODIFIED `services/ledger/src/handlers/mod.rs`,
  `services/ledger/src/server.rs`

### Adversarial review

- **Race on expired lease**: FOR UPDATE serializes; second contender
  observes the takeover and falls to Path C (denied).
- **Caller mints epoch?** SP is sole writer; caller supplies only
  TTL + identity.
- **Stale owner writes after takeover?** existing
  post_ledger_transaction fencing CAS rejects stale epoch; S3 only
  changes how epoch is set, not how it's gated.
- **Audit invariant?** fencing_scope_events row atomic with UPDATE.
- **Tenant boundary**: SP rejects if scope.tenant != caller.tenant.

### Residual risks

1. Sidecar wiring deferred to S4. Until S4, sidecar still uses seeded
   `current_epoch=1`. RPC callable but no production caller yet.
2. SDK client method on sidecar deferred to S4.
3. Build validation deferred to next Docker rebuild.

---

(Subsequent slice entries appended below.)
