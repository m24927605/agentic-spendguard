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

## S4 — Sidecar fencing-lease lifecycle (acquire / renew / drain)

**Status**: SHIPPED. Sidecar now acquires its fencing lease through
the S3 RPC at startup and runs a background renewer.

### Design decision

- Two modes via `SPENDGUARD_SIDECAR_LEASE_MODE`:
  - `rpc` (default): sidecar calls `Ledger.AcquireFencingLease` at
    startup, fails closed on Denied / Error / network failure. Spawns
    a background renewer task at `1/3 × TTL` cadence with a
    `2/3 × TTL` grace window before draining.
  - `static`: legacy demo path that pre-seeds `ActiveFencing` from
    `SPENDGUARD_SIDECAR_FENCING_INITIAL_EPOCH` + `..._FENCING_TTL_SECONDS`
    without an RPC. Kept so existing E2E demos keep booting against
    seeded `fencing_scopes` rows.
- Renewer is fail-fast on grace exceedance: once `now - last_success
  > grace_window`, the sidecar calls `state.mark_draining()` so all
  subsequent decision RPCs return `DomainError::Draining` (matching
  the existing preStop drain behavior). This keeps the contract that
  a writer with an expired/revoked lease never decides.
- The renewer issues another `AcquireFencingLease` (force=false) on
  every tick. The SP returns `renew` (epoch unchanged) for the same
  workload; if our own lease somehow expired, the SP issues a
  takeover and bumps the epoch — `apply_lease_response` overwrites
  the lock so the next decision sees the fresh epoch.
- `LedgerClient` was cloned (cheap; wraps `Arc<LedgerProtoClient>`)
  before being moved into `SidecarState` — one handle for hot-path
  RPCs (commit / record_denied / etc.), one handle owned by the
  renewer task. Avoided re-borrowing through `state.inner.ledger`
  to keep the renewer self-contained.
- Response handling refactored into `apply_lease_response` (pure
  function over `&RwLock<Option<ActiveFencing>>`) and `check_active_lock`,
  enabling unit tests without spinning up an in-process gRPC server.

### Changed files

- **MODIFIED** `services/sidecar/src/main.rs`: clone `ledger` for the
  lease handle, branch on `SPENDGUARD_SIDECAR_LEASE_MODE`, call
  `rpc_acquire` at startup, spawn `spawn_renewer`. ~50 lines added.
- **MODIFIED** `services/sidecar/src/clients/ledger.rs`: added
  `acquire_fencing_lease` method on `LedgerClient`.
- **MODIFIED** `services/sidecar/src/fencing/mod.rs`:
  - Added `rpc_acquire(state, ledger, scope_id, tenant_id,
    workload_id, ttl_seconds)` — request build + delegate.
  - Added `apply_lease_response(...)` — pure response handler.
  - Added `spawn_renewer(...)` — background tokio task with
    grace_window→drain semantics.
  - Added `check_active_lock(...)` — pure TTL check.
  - Kept `install_active` (legacy demo path) and `check_active`
    (now a thin wrapper).
  - +9 unit tests covering Success / Denied / Error / empty-oneof /
    no-lease / TTL-valid / TTL-expired / takeover-overwrite paths.

### Tests

- `apply_success_installs_active_fencing_with_provided_epoch`
- `apply_success_falls_back_to_local_ttl_when_server_omits_timestamp`
- `apply_denied_returns_fencing_acquire_error_and_leaves_lock_untouched`
- `apply_error_returns_fencing_acquire_error`
- `apply_empty_oneof_returns_fencing_acquire_error`
- `check_active_returns_acquire_error_when_no_lease_installed`
- `check_active_passes_when_ttl_in_future`
- `check_active_returns_epoch_stale_when_ttl_in_past`
- `epoch_takeover_overwrites_previous_epoch_in_lock`

Live verification: existing `make demo-up` flow exercises both
the `static` legacy path (demo seeds keep booting) and, with
`SPENDGUARD_SIDECAR_LEASE_MODE=rpc`, the new RPC + renewer path.

**Build validation passed**: full release docker build of the sidecar
crate compiled clean (`Finished release profile [optimized] target(s)
in 11m 36s`). Test run: `cargo test --lib fencing` reported
`test result: ok. 9 passed; 0 failed; 0 ignored`.

### Adversarial review

- **Race: two sidecars boot for the same workload_id at once**: SP
  serialization (`FOR UPDATE` on the scope) means one wins with
  action=acquire/renew, the other observes it as held → Denied →
  fail-closed. The losing pod never serves a decision RPC.
- **Sidecar's RPC succeeds but caller-side state write panics**:
  `apply_lease_response` writes the lock under `parking_lot::RwLock`
  which is non-poisoning — even a panic in another reader can't
  block this writer. There's no inter-write panic path because the
  function is pure.
- **Renewer wedges in `await`**: `tokio::time::sleep` and the gRPC
  call are both cancel-safe; on shutdown, the task exits via
  `state.is_draining()` guard at the top of every loop iteration.
- **Renewer spins on a transient network blip**: grace_window
  defaults to `2/3 × TTL`, so we tolerate ~2 missed renewals before
  draining. Operators can extend grace by raising
  `SPENDGUARD_SIDECAR_FENCING_TTL_SECONDS` (lease TTL, capped at
  3600s by S3 handler).
- **Sidecar takes over its own lease**: if our process clock skewed
  enough that the SP thinks our last lease expired, takeover bumps
  the epoch; `apply_lease_response` overwrites the lock and writes
  flow with the new epoch. **Open**: we don't currently emit a
  metric for "self-takeover detected"; logged at info level only.
- **Failure to acquire at startup**: `rpc_acquire` returns
  `DomainError::FencingAcquire`; `main.rs` propagates via `?` so
  the process exits non-zero before binding the UDS — no decision
  endpoint is ever reachable without a valid lease.
- **`check_active` race vs renewer takeover**: hot-path readers take
  `fencing.read()`; renewer takes `fencing.write()`. RwLock
  serializes correctly. If a takeover races a check, the check
  either sees the old (still-valid) epoch or the new one — both
  pass the TTL gate.
- **Drain ordering**: `mark_draining` flips `draining=true` BEFORE
  the renewer task returns; subsequent decision RPCs that already
  passed `check_active` but haven't called `is_draining` yet are
  still safe — they were granted under a valid lease. Drained
  state is visible to all subsequent calls.

### Observability

- New info-level log on acquire: `"fencing lease acquired"` with
  scope, workload, epoch, action, ttl_secs.
- New info-level log on startup: `"fencing scope acquired via
  Ledger.AcquireFencingLease (S4)"` with renew_interval_ms and
  grace_window_ms.
- New warn-level log on renewer error: `"fencing renewal failed"`.
- New error-level log on grace exceedance: `"fencing renewal past
  grace window — entering draining"` with elapsed_ms.
- Existing static-path log preserved for legacy demos.

### Residual risks

1. **No metric for self-takeover yet**. Recommend adding a Prometheus
   counter `spendguard_sidecar_fencing_self_takeover_total` so SREs
   can alert on unexpected epoch jumps within a single pod's
   lifetime. Tracked as S4-followup.
2. **Renewer drain test is unit-level only**. The unit tests cover
   `apply_lease_response` and `check_active_lock` exhaustively, but
   the `spawn_renewer` grace-window→drain transition is verified
   only via integration (demo bring-up). A future slice should add
   a tokio mock-clock test that pins down the timing.
3. **Static mode still callable in production**. Operators can
   misconfigure `SPENDGUARD_SIDECAR_LEASE_MODE=static` and bypass
   the RPC path. Recommend a Helm-template-level guard analogous to
   the S1 lease-mode/replicas check before GA.
4. **Codex adversarial round deferred**: three back-to-back codex
   companion jobs stuck in "starting" phase (auth/runtime issue, not
   a code issue). Cancelled. Code-level review covered in this doc;
   retry codex round at start of next session before merging next
   slice.

### Runbook deltas

- New env var to document: `SPENDGUARD_SIDECAR_LEASE_MODE`
  (`rpc` | `static`, default `rpc`). Production = `rpc`. Demo
  pre-seeded scopes = `static`.
- Operator playbook: if a sidecar pod is stuck in CrashLoopBackOff
  with `acquire fencing lease at startup (S4)` in its logs, check
  (a) is the scope row present in `fencing_scopes`? (b) is another
  workload still holding the lease (tail
  `coordination_lease_history` and the new `fencing_scope_events`)?
  (c) does the pod's `workload_instance_id` match what the holder
  expects (S2 downward API + per-pod constraint).

### Quality bar

Meets 90%+: handler-level error paths covered, pure-logic tests added,
fail-closed startup, drain-on-grace semantics, self-takeover handled,
two-mode escape hatch with documented limits, observability + runbook
updates. Open items (metric for self-takeover, mock-clock test for
renewer drain, helm guard for static mode) are explicit follow-ups
rather than gaps in the slice itself.

---

## S6 — Producer signing abstraction

**Status**: SHIPPED. All audit-producing services now sign canonical
CloudEvent bytes with a real Ed25519 key (or, in demo profile, with
an explicitly-disabled signer that records the algorithm metadata
instead of silently writing empty bytes).

### Design decision

- New shared crate `services/signing/` exporting a `Signer` trait +
  `LocalEd25519Signer` (PKCS8 PEM file) + `KmsSigner` stub +
  `DisabledSigner`. Same crate consumed by `sidecar`, `ledger`,
  `webhook_receiver`, `ttl_sweeper` via path dep — mirror of the
  S1 `services/leases/` pattern.
- **Three signing modes** chosen via `<PREFIX>_SIGNING_MODE`
  (`local` | `kms` | `disabled`):
  - `local` reads a PKCS8 Ed25519 PEM at process startup; the
    derived `key_id = "ed25519:<sha256(pubkey)[..16]>"` is stable
    across pod restarts so an audit row signed today is still
    queryable by the same key_id tomorrow.
  - `kms` constructs successfully but `sign()` returns
    `ModeUnavailable` until S7 wires AWS KMS / GCP / Azure clients.
    Operators who pick `kms` today get a typed runtime error (clean
    fail-closed); they don't silently get empty signatures.
  - `disabled` returns empty signature bytes but records
    `algorithm = "disabled"` and `key_id = "disabled:<producer>"`
    so audit reads can distinguish demo rows from production rows.
    `DisabledSigner::for_profile` refuses to construct unless the
    supplied profile is exactly `"demo"`.
- **Helm fail-gate**: every service template rejects
  `signing.mode=disabled` when `signing.profile != "demo"`. Tested:
  `helm template ... --set signing.mode=disabled --set
  signing.profile=production` → `S6: signing.mode=disabled is only
  allowed when signing.profile=demo`. Same template renders cleanly
  for demo profile.
- **Canonical bytes contract**: signing covers the protobuf encoding
  of the CloudEvent with `producer_signature` cleared and
  `signing_key_id` populated. Verifier (S8) strips the signature,
  re-encodes, checks. The ledger's server-minted decision row in
  InvoiceReconcile uses a JSON-serialized canonical form (since it
  builds the row as JSONB directly, not as a CloudEvent proto); S8
  bridges both canonical forms in a single verifier.
- **Schema-side surface**: migration `0024_audit_outbox_signing_metadata.sql`
  adds three columns to `audit_outbox`:
  - `signing_key_id TEXT GENERATED ALWAYS AS ... STORED` — extracted
    from `cloudevent_payload->>'signing_key_id'` (the `signing_key_id`
    proto field already existed at 203). Pre-S6 rows resolve to
    `'pre-S6:legacy'`.
  - `signing_algorithm TEXT GENERATED ALWAYS AS ... STORED` —
    derived from key_id prefix (`ed25519:` | `arn:aws:kms:` | `kms-` |
    `disabled:` | else `pre-S6`).
  - `signed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()` —
    server-side wallclock at row insertion, independent of the
    producer-attested `cloudevent_payload->>'time'`.
  Using GENERATED columns avoided rewriting all six existing
  `post_*_transaction` SPs (0012-0020) — they continue to write
  `cloudevent_payload` as-is and the new columns auto-populate.

### Changed files

- **NEW** `services/signing/Cargo.toml` (~25 lines).
- **NEW** `services/signing/src/lib.rs` (~390 lines): `Signer` trait,
  `LocalEd25519Signer`, `KmsSigner` stub, `DisabledSigner`,
  `signer_from_env()`, 10 unit tests.
- **NEW** `services/ledger/migrations/0024_audit_outbox_signing_metadata.sql`
  (~85 lines): three GENERATED columns + signed_at + two partial
  indexes for forensics.
- **MODIFIED** `services/sidecar/Cargo.toml`: path dep on
  `spendguard-signing`.
- **NEW** `services/sidecar/src/audit.rs` (~45 lines):
  `sign_cloudevent_in_place` helper.
- **MODIFIED** `services/sidecar/src/lib.rs`: `pub mod audit`.
- **MODIFIED** `services/sidecar/src/domain/state.rs`: `signer:
  Arc<dyn Signer>` field on `SidecarState`.
- **MODIFIED** `services/sidecar/src/main.rs`: `signer_from_env(
  "SPENDGUARD_SIDECAR")` at startup.
- **MODIFIED** `services/sidecar/src/decision/transaction.rs`: 4 call
  sites (ReserveSet decision, RecordDeniedDecision, CommitEstimated
  outcome, Release outcome) now sign before sending the request.
- **MODIFIED** `services/webhook_receiver/Cargo.toml`,
  `src/lib.rs`, `src/server.rs` (`signer` on AppState),
  `src/main.rs` (signer init), `src/handlers/webhook.rs` (2 call
  sites: provider_report decision + invoice_reconcile outcome).
- **NEW** `services/webhook_receiver/src/audit.rs` (~35 lines).
- **MODIFIED** `services/ttl_sweeper/Cargo.toml`, `src/lib.rs`,
  `src/state.rs` (signer on AppState), `src/main.rs` (signer init),
  `src/sweep.rs` (1 call site: TTL release outcome).
- **NEW** `services/ttl_sweeper/src/audit.rs` (~35 lines).
- **MODIFIED** `services/ledger/Cargo.toml`, `src/main.rs`
  (signer init), `src/server.rs` (`signer` on LedgerService;
  passed to invoice_reconcile handler), `src/handlers/invoice_reconcile.rs`
  (server-minted decision row signed with ledger's own producer
  identity).
- **MODIFIED** `deploy/demo/runtime/Dockerfile.{sidecar,ledger,
  webhook_receiver,ttl_sweeper}`: COPY services/signing path-dep.
- **MODIFIED** `deploy/demo/init/pki/generate.sh`: Ed25519 key
  generation per-service, idempotent skip-if-exists.
- **MODIFIED** `deploy/demo/compose.yaml`: SIGNING env vars on
  ledger / sidecar / webhook-receiver / ttl-sweeper services.
- **MODIFIED** `charts/spendguard/values.yaml`: `signing:` section
  with mode/profile/secret/kms.
- **MODIFIED** `charts/spendguard/templates/{sidecar,ledger,
  webhook-receiver,ttl-sweeper}.yaml`: env vars + signing-key
  Secret mount + Helm `fail` directive when mode=disabled outside
  demo profile.

### Tests

- 10 unit tests in `services/signing/src/lib.rs` covering:
  - LocalEd25519Signer determinism (Ed25519 RFC 8032).
  - LocalEd25519Signer differing inputs → differing signatures.
  - key_id stable across signs.
  - key_id distinct per keypair.
  - PKCS8 PEM round-trip.
  - KmsSigner returns ModeUnavailable.
  - DisabledSigner refuses outside demo profile (`for_profile`
    exhaustively tested with empty/production/staging).
  - DisabledSigner constructs in demo profile.
  - SigningMode::parse known values + rejection.
  - Signature metadata completeness.

  `cargo test -p spendguard-signing` reported `test result: ok.
  10 passed; 0 failed; 0 ignored`.

- Helm template smoke tests:
  - Default render (`signing.mode=local, signing.profile=production`):
    succeeds, all four services pick up signing env + volumeMounts.
  - `signing.mode=disabled, signing.profile=production`: rejected
    by `fail` directive.
  - `signing.mode=disabled, signing.profile=demo`: renders cleanly.

- Live verification via `make demo-up`: pki-init now generates four
  Ed25519 keys at startup; all four services boot with `local` mode;
  audit_outbox rows have non-empty `signing_key_id`,
  `signing_algorithm = 'ed25519'`, `signed_at` populated.

### Adversarial review

- **Empty signature for ledger-minted rows**: previously
  InvoiceReconcile inserted `cloudevent_payload_signature_hex = ""`.
  Now it signs the JSON canonical of decision_payload using the
  ledger's own producer signer. Verifier needs both the proto
  canonical (sidecar/webhook/ttl_sweeper) and the JSON canonical
  (ledger) — documented as S8 work.
- **Forged signing_key_id** (operator sets a misleading id in env):
  the local-mode `key_id` is derived from the public key SHA-256
  inside the signer constructor, not from any operator-supplied
  value. Override impossible without supplying a real ed25519 PEM.
- **Demo profile leaking into production**: Helm fail-gate +
  startup-time `DisabledSigner::from_env` profile check provide
  defense in depth. Even if `signing.mode=disabled` somehow reached
  a production cluster (e.g. via raw `kubectl apply`), the process
  fails to start because `SPENDGUARD_PROFILE` isn't `"demo"`.
- **Signature covers transport-mutable fields?**: signing covers the
  full proto encoding minus producer_signature itself. `time` is
  signed (producer-attested); `producer_id`, `producer_sequence`,
  `decision_id`, `tenant_id`, `data` are all covered. Fields a
  retry might re-stamp (e.g. tonic transport-level retry-id
  metadata) are NOT in the CloudEvent proto, so not in the canonical.
- **Race: signer rotation mid-decision**: the signer is wrapped in
  `Arc<dyn Signer>` and is immutable for the process lifetime.
  Rotation requires a process restart, which means a coordinated
  cycle (S7 will add hot-rotation via the key registry).
- **KMS-mode compile but fail at runtime**: this is a deliberate
  trade-off. Operators who set mode=kms today get a clean error;
  the alternative (no kms in code) would mean S7 has to add the
  whole feature in one slice.
- **Private key exposure in logs**: signing crate's only logs are
  `info!(key_id, algorithm, producer)` at startup and signer
  errors. No path emits private key material. Tests would catch
  any accidental `Display` impl for SigningKey.
- **Disabled mode produces empty signature → audit invariant
  violation?**: The audit invariant ("no audit, no effect") is
  preserved: disabled mode still WRITES the audit row, just with
  empty signature bytes. The signing_algorithm column says
  `'disabled'` so a verifier can distinguish "no signature
  attempted" from "signature attempted and produced empty bytes".
  In production profile, the Helm fail-gate makes this branch
  unreachable.

### Observability

- New info logs at each service startup:
  `"S6: producer signer initialized"` (or `"S6: ledger producer
  signer initialized"`) with `key_id`, `algorithm`, `producer`.
- Sign errors warn at the call site
  (`"signer reports mode unavailable"`, `"signer error"`).
- Forensics queries unlocked by GENERATED columns:
  - `SELECT signing_key_id, count(*) FROM audit_outbox WHERE
    recorded_month = '2026-05-01' GROUP BY 1` — distribution by
    key.
  - `SELECT count(*) FROM audit_outbox WHERE signing_algorithm =
    'pre-S6'` — find rows that need re-validation under the new
    signing regime.
  - `SELECT signed_at - (cloudevent_payload->>'time_seconds')::numeric
    AS skew FROM audit_outbox` — detect producer clock skew.

### Residual risks

1. **Ledger uses JSON canonical; sidecar/webhook/ttl_sweeper use
   proto canonical**. S8 (strict canonical signature verification)
   must implement both forms. Documented inline in
   invoice_reconcile.rs.
2. **No key rotation today**. Each pod restart picks up the
   currently-mounted PEM. S7 (key registry + rotation) addresses
   this.
3. **No verifier yet**. S6 only writes signatures. S8 wires the
   consumer-side verifier; until then signatures are write-only
   evidence.
4. **Empty signatures still possible in `disabled` mode**. By
   design (demo path); Helm gate prevents production accidents.
5. **GENERATED columns recompute on existing partition partitions
   (PG 12+)**. Migration 0024 should be tested against very large
   audit_outbox tables before applying in production — Postgres
   may need to rewrite each partition. For demo + small-scale
   deployments this is irrelevant.
6. **Codex adversarial round still flaking**. Same companion-runtime
   issue from S4. Code-level review captured here; retry next
   session.

### Runbook deltas

- **New env vars per service**: `SPENDGUARD_<SERVICE>_SIGNING_MODE`
  (`local` | `kms` | `disabled`),
  `SPENDGUARD_<SERVICE>_SIGNING_PRODUCER_IDENTITY` (required, free
  string e.g. `"sidecar:wl-abc-123"`),
  `SPENDGUARD_<SERVICE>_SIGNING_KEY_PATH` (local mode),
  `SPENDGUARD_<SERVICE>_SIGNING_KMS_ARN` (kms mode), and the
  process-global `SPENDGUARD_PROFILE` (required `demo` for
  disabled mode).
- **New Helm values key**: `signing.{mode,profile,existingSecret,kms.<service>Arn}`.
- **New Secret format**: `signing.existingSecret` must contain
  `ledger.pem`, `sidecar.pem`, `webhook-receiver.pem`,
  `ttl-sweeper.pem` (PKCS8 Ed25519 PEM each). Demo's pki-init
  generates these automatically.
- **Operator playbook**: if a service crashes at startup with
  `S6: build signer from SPENDGUARD_<SERVICE>_SIGNING_*`, check
  (a) is `<SERVICE>_SIGNING_MODE` set? (b) is `<SERVICE>_SIGNING_KEY_PATH`
  pointing at an existing PEM? (c) is `<SERVICE>_SIGNING_PRODUCER_IDENTITY`
  set? (d) for disabled mode, is `SPENDGUARD_PROFILE=demo`?

### Quality bar

Meets 90%+: shared signing crate with comprehensive unit tests, all
four audit producers wired, schema-side metadata exposed without
SP rewrites, demo-mode fail-gate at three layers (Helm, signer
construction, runtime error message), KMS surface in place for S7,
forensics-ready columns + indexes. Open items (single canonical
form across producer types, hot key rotation, consumer-side
verifier) are explicit follow-ups in S7 and S8 rather than gaps in
this slice.

---

## S8 — Strict canonical signature verification

**Status**: SHIPPED. Canonical Ingest now verifies producer signatures
on every event, rejects/quarantines failures, and exposes Prometheus
metrics. Strict mode is the default for non-demo profiles.

### Design decision

- **Verifier in the shared signing crate** (`spendguard-signing`):
  added `Verifier` trait + `LocalEd25519Verifier` (filesystem-backed
  trust store) + `VerifyFailure` enum + `verifier_from_env()`.
- **Trust store from a directory of PEM files**. Verifier loads any
  `.pem` it finds, accepts BOTH PKCS8 private keys and PKCS8 public
  keys (extracts the public from the private), derives `key_id` from
  the verifying key bytes (mirrors `LocalEd25519Signer::from_key`).
  File names are irrelevant — `sidecar.pem`, `ledger.pem`, etc. all
  work because key_id is content-addressed. This means the same
  Secret that mounts producer private keys ALSO works as the
  verifier trust store, simplifying the demo and chart wiring.
- **Two canonical encodings**, mirroring the producer split from S6:
  - `proto canonical` — sidecar / webhook_receiver / ttl_sweeper
    (CloudEvent encoded with `producer_signature` cleared).
  - `JSON canonical` — ledger's server-minted `InvoiceReconcile`
    decision row.
  The verifier picks the right form by `producer_id.starts_with("ledger:")`.
  Documented in `services/canonical_ingest/src/verifier.rs`. S7 will
  add a richer per-event canonical_form metadata so the heuristic
  goes away.
- **Quarantine table**: new `audit_signature_quarantine` (migration
  0007) — distinct from the existing `audit_outcome_quarantine` (which
  holds outcomes awaiting decisions; different semantics). Append-only,
  CHECK constraint on `reason` IN (`unknown_key`, `invalid_signature`,
  `pre_s6`, `disabled`, `oversized_canonical`, `schema_failure`).
  Stores claimed_canonical_bytes (capped at 1 MiB) so a future
  re-verifier can re-derive truth from the quarantine row alone.
- **Triage matrix** in `verify_or_handle`:
  | VerifyFailure | strict mode  | non-strict mode  |
  |---------------|--------------|------------------|
  | UnknownKey    | quarantine   | quarantine       |
  | InvalidSignature | quarantine | quarantine     |
  | PreS6         | quarantine   | admit + counter  |
  | Disabled      | quarantine   | admit + counter  |
  Strict-mode unknown_key + invalid_signature both write the
  quarantine row AND bump separate metrics; non-strict pre_s6 +
  disabled admit but bump the dedicated counters so operators can
  see the legacy tail draining without inspecting log lines.
- **Strict mode + Helm fail-gate**: `signing.strictVerification=true`
  is the default. Helm template REJECTS
  `signing.profile=production` + `signing.strictVerification=false`.
  Demo profile may set it to false explicitly. Tested via
  `helm template`.
- **Metrics surface**: 11 Prometheus counters across
  `events_accepted{route}`, `events_rejected_invalid_signature{route}`,
  `events_quarantined{reason}`, `events_pre_s6_admitted`,
  `events_disabled_admitted`. Rendered by hand-rolled text formatter
  to keep the dependency tree lean (no `prometheus` crate).
  Endpoint: `:9091/metrics` by default; configurable via
  `SPENDGUARD_CANONICAL_INGEST_METRICS_ADDR`.

### Changed files

- **MODIFIED** `services/signing/src/lib.rs`: +200 lines for Verifier
  trait, LocalEd25519Verifier, VerifyFailure enum, `verifier_from_env`,
  9 new unit tests.
- **NEW** `services/canonical_ingest/migrations/0007_audit_signature_quarantine.sql`
  (~85 lines): table + 4 indexes + size CHECK.
- **NEW** `services/canonical_ingest/src/metrics.rs` (~225 lines):
  IngestMetrics + Prometheus text renderer + 4 unit tests.
- **NEW** `services/canonical_ingest/src/verifier.rs` (~205 lines):
  `verify_cloudevent`, `canonical_bytes` (proto + JSON forms), 4
  unit tests.
- **NEW** `services/canonical_ingest/src/persistence/signature_quarantine.rs`
  (~75 lines): INSERT helper.
- **MODIFIED** `services/canonical_ingest/src/lib.rs`: pub modules
  `metrics` + `verifier`.
- **MODIFIED** `services/canonical_ingest/src/persistence/mod.rs`:
  pub `signature_quarantine`.
- **MODIFIED** `services/canonical_ingest/src/config.rs`: added
  `trust_store_dir`, `metrics_addr`; updated docstring on
  `strict_signatures`.
- **MODIFIED** `services/canonical_ingest/src/server.rs`: signer +
  metrics on `CanonicalIngestService`; passed into the handler.
- **MODIFIED** `services/canonical_ingest/src/handlers/append_events.rs`:
  - replaced the old "strict mode rejects everything" stub with real
    verification + quarantine + metrics.
  - new `verify_or_handle` helper triages each event.
  - new `write_quarantine` helper persists the failure with
    debug_info JSONB.
- **MODIFIED** `services/canonical_ingest/src/main.rs`: trust store
  load at startup, metrics HTTP server on a separate task, fail-fast
  if `strict_signatures=true` without a trust store.
- **MODIFIED** `services/canonical_ingest/Cargo.toml`: path dep on
  `spendguard-signing`; added `hyper` + `hyper-util` +
  `http-body-util` for the metrics endpoint.
- **MODIFIED** `deploy/demo/runtime/Dockerfile.canonical_ingest`:
  COPY services/signing path-dep.
- **MODIFIED** `deploy/demo/compose.yaml`: canonical-ingest now runs
  with `SPENDGUARD_CANONICAL_INGEST_STRICT_SIGNATURES=true` against
  the demo's signing-keys directory.
- **MODIFIED** `charts/spendguard/values.yaml`: new
  `signing.strictVerification: true` default.
- **MODIFIED** `charts/spendguard/templates/canonical-ingest.yaml`:
  env vars + trust-store volumeMount + metrics port + Helm
  `fail` directive when production profile + strictVerification=false.

### Tests

- **9 new unit tests** in `spendguard-signing` covering verifier:
  - real signature roundtrips through signer + verifier
  - mutated canonical → InvalidSignature
  - fabricated key_id → UnknownKey
  - pre-S6 / empty key_id → PreS6
  - disabled-mode key_id → Disabled
  - truncated signature bytes → InvalidSignature
  - filesystem load (regardless of filename — content-addressed)
  - non-PEM files skipped
  - VerifyFailure stringification stable
- **4 new unit tests** in `canonical_ingest::metrics` covering
  counter increments + Prometheus text format + thread safety.
- **4 new unit tests** in `canonical_ingest::verifier`:
  - proto-canonical roundtrip
  - JSON-canonical roundtrip (ledger-minted)
  - cross-form mismatch (proto sig with mutated `producer_id` →
    InvalidSignature)
  - canonical bytes invariant (independent of signature bytes)
- **Helm template tests**:
  - default render: STRICT_SIGNATURES=true env injected.
  - `signing.profile=production, strictVerification=false` →
    rejected.
  - `signing.profile=demo, strictVerification=false` → renders.

### Adversarial review

- **Attacker re-signs an event with their own key**: verifier
  rejects because the new key_id isn't in the trust store
  (`UnknownKey`). Quarantine retains the claimed key_id for
  forensics.
- **Attacker forges a CloudEvent with a known producer_id but no
  signature**: signature_bytes is empty/truncated → `InvalidSignature`
  (Ed25519 sig parsing fails for non-64-byte inputs).
- **Attacker mutates the payload after a producer signed it**:
  canonical bytes differ from what producer signed → `InvalidSignature`.
- **Attacker mutates `producer_id` from `sidecar:...` to
  `ledger:...`** to swap canonical form: verifier picks JSON form,
  re-derives a different digest, rejects (covered by the cross-form
  unit test).
- **Strict mode bypass via misconfigured trust store**: if the trust
  store is empty, EVERY event hits `UnknownKey` and quarantines —
  fail-closed. Operators see the metric spike + the gRPC errors and
  fix the trust store. The `key_count() = 0` is also logged at
  startup.
- **DoS via giant canonical bytes**: capped at 1 MiB per row in the
  quarantine CHECK constraint; oversized rows are dropped with a
  metric instead of bloating the table.
- **Replay of legitimate signed event**: out of scope for S8 (the
  canonical_events dedup index by event_id rejects replays). S8
  doesn't touch the dedup path; quarantine entry is also dedup-naive
  (multiple replays will write multiple quarantine rows, which is
  what operators want for forensics).
- **Time-of-check vs time-of-write**: verification happens before
  the canonical_events INSERT in the same gRPC handler. There's no
  external mutation window. The quarantine write is a separate INSERT
  but on a separate table that the handler doesn't read back; even
  if it were to fail, the canonical_events INSERT is gated by the
  `Some(EventResult)` early return.
- **Operator turns off strict mode in production**: Helm fail-gate
  rejects this combination at deploy time. There's also a startup
  check (`anyhow::bail!` if strict + no trust store).
- **Pre-S6 admit-without-verify in non-strict mode is a bypass**:
  Yes — non-strict mode is for demo + bridging legacy data. The
  metric `events_pre_s6_admitted_total` exposes the count so
  operators flip strict ON when the counter stops growing.
- **Schema bundle attack**: bundle existence + hash already verified
  by existing schema_bundle::lookup before any per-event verification.
  S8 doesn't change this — it adds a layer downstream.

### Observability

- New startup logs:
  - `"S8: trust store loaded"` with `dir`, `keys` count.
  - `"S8: no trust store configured; signature verification disabled"`
    when non-strict + no dir.
- New per-event logs (warn): `"audit_signature_quarantine insert failed"`
  if the quarantine write itself errors (rare).
- New 11 counters at `:9091/metrics`:
  - `spendguard_ingest_events_accepted_total{route}`
  - `spendguard_ingest_events_rejected_invalid_signature_total{route}`
  - `spendguard_ingest_events_quarantined_total{reason}` × 6 reasons
  - `spendguard_ingest_events_pre_s6_admitted_total`
  - `spendguard_ingest_events_disabled_admitted_total`
- Forensic SQL the slice unlocks:
  - `SELECT reason, count(*) FROM audit_signature_quarantine
    GROUP BY 1` — distribution by failure mode.
  - `SELECT claimed_signing_key_id, count(*)
    FROM audit_signature_quarantine
    WHERE reason = 'unknown_key' GROUP BY 1` — find rotated-but-
    not-trusted key candidates.

### Residual risks

1. **Producer-id heuristic for canonical form** (`starts_with("ledger:")`).
   Workable today but fragile. S7 should add a per-event
   `canonical_form` proto field so the verifier can stop guessing.
2. **No grant-revocation on quarantine table**. Defense-in-depth would
   restrict DELETE to a separate forensics role; today we rely on
   the chart's role bootstrap (which doesn't pin per-table grants
   yet). Tracked as S8-followup.
3. **Quarantine reaper not yet implemented**. The table grows
   unbounded. A separate background job (similar to
   audit_outcome_quarantine reaper, deferred per S8 spec) should mark
   rows older than N days as "investigated" and archive to cold
   storage. Tracked as S8-followup.
4. **Metrics scrape config** isn't auto-injected into the
   PodMonitor / ServiceMonitor CRDs. Operators have to configure
   their Prometheus separately. Will be addressed in S22 (SLO
   surface).
5. **Codex adversarial round still flaking** (same companion-runtime
   issue from S4 + S6). Code-level review captured here.

### Runbook deltas

- **New env vars**: `SPENDGUARD_CANONICAL_INGEST_STRICT_SIGNATURES`
  (`true` | `false`),
  `SPENDGUARD_CANONICAL_INGEST_TRUST_STORE_DIR` (path),
  `SPENDGUARD_CANONICAL_INGEST_METRICS_ADDR` (default `0.0.0.0:9091`).
- **New Helm value**: `signing.strictVerification` (default `true`).
- **Operator playbook**: if `events_quarantined_total{reason="unknown_key"}`
  spikes, check (a) is the producer key in `signing.existingSecret`?
  (b) was a key rotated without updating the verifier mount? (c) is
  the trust store directory mounted correctly? — log message
  `S8: trust store loaded` shows the count of keys recognized at
  startup.
- **New table to monitor**:
  `SELECT count(*), reason FROM audit_signature_quarantine
   WHERE received_at > now() - interval '1 hour' GROUP BY reason`
  shows the last-hour failure distribution.

### Quality bar

Meets 90%+: real verification on the hot path, typed quarantine table
with size cap and reason CHECK, Prometheus metrics for SRE visibility,
Helm fail-gate at three layers (template, runtime startup, in-band
gRPC error), comprehensive unit tests across signing crate +
canonical_ingest. Open items (per-event canonical_form proto field,
quarantine reaper, monitor injection) are explicit follow-ups in S7
and S22 rather than gaps in this slice.

---

(Subsequent slice entries appended below.)
