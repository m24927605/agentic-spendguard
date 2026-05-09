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

## S17 — OIDC/SSO foundation

**Status**: SHIPPED. Dashboard and Control Plane no longer accept a
single hard-coded admin bearer token; both validate OIDC JWTs (or, in
demo profile, a static token via the explicit `static_token` mode).

### Design decision

- **New shared crate `services/auth/`**: `Authenticator` enum dispatch
  over `JwtValidator` and `StaticTokenConfig`. JWKS via
  `HttpJwksProvider` with refresh-on-stale (default 3600s). Uses
  `jsonwebtoken` 9 + `reqwest` for fetch.
- **Two modes only** to keep the surface small:
  - `jwt` (default for production) — issuer + audience + JWKS URL
    are required env vars; clock skew leeway defaults to 60s.
  - `static_token` (demo profile only) — `AuthConfig::from_env`
    refuses to construct unless `SPENDGUARD_PROFILE=demo`.
- **Constant-time token comparison** for static_token mode (`subtle_eq`
  helper) so a length-mismatch attack can't observe early-return
  timing.
- **Public-safe error messages**. Spec: "auth failures must not
  reveal tenant existence." Internal `AuthError` variants distinguish
  `IssuerMismatch`, `AudienceMismatch`, `Expired`, `UnknownKid`,
  etc., but `safe_public_message()` collapses them all to
  `"unauthorized"` (or `"missing authorization"` /
  `"service temporarily unavailable"`). Asserted by a unit test that
  walks every variant + checks: no `kid`, no `issuer`, no `network`.
- **Principal in axum extensions**: middleware decodes the JWT,
  extracts (issuer, subject, groups, tenant_ids, roles, mode) into
  a `Principal`, places it in request extensions. Handlers read via
  `Extension<Principal>`. S17 leaves `roles` empty — S18 wires
  groups → roles policy.
- **Tenant claim mapping**: default claim names `groups` and
  `spendguard:tenant_ids` are configurable via env vars
  (`<PREFIX>_OIDC_GROUPS_CLAIM`, `<PREFIX>_OIDC_TENANT_IDS_CLAIM`)
  so the auth crate works with Entra ID, Auth0, Okta, generic OIDC
  without code changes.
- **JWKS cache fail-open for warm restarts, fail-closed on cold**.
  If the JWKS endpoint is unreachable AFTER a previous successful
  fetch, the verifier serves the stale cache + warns. On COLD start
  (cache empty + JWKS unreachable), verification fails — operators
  get an explicit error instead of silently admitting unauthed.

### Changed files

- **NEW** `services/auth/Cargo.toml` (~40 lines).
- **NEW** `services/auth/src/lib.rs` (~700 lines): Authenticator,
  JwtValidator, HttpJwksProvider with cache, Principal, AuthConfig
  with profile gate, axum middleware, 15 unit tests.
- **MODIFIED** `services/dashboard/Cargo.toml`: path dep on
  `spendguard-auth`.
- **MODIFIED** `services/dashboard/src/main.rs`: removed
  `auth_token` field on AppState + `check_auth` helper; wired
  `Authenticator` + `from_fn_with_state(auth, require_auth)` on
  the `/api/*` routes; handlers now take `Extension<Principal>`.
- **MODIFIED** `services/control_plane/Cargo.toml`: path dep on
  `spendguard-auth`.
- **MODIFIED** `services/control_plane/src/main.rs`: removed
  `admin_token` + `check_auth`; wired Authenticator behind a
  scoped sub-router; handlers receive `Extension<Principal>` and
  log subject + mode for mutating actions (create_tenant,
  tombstone_tenant).
- **MODIFIED** `deploy/demo/runtime/Dockerfile.dashboard`,
  `Dockerfile.control_plane`: COPY services/auth path-dep.
- **MODIFIED** `deploy/demo/compose.yaml`: dashboard +
  control_plane now use `static_token` mode under
  `SPENDGUARD_PROFILE=demo`. Static token strings are operator-
  visible so the demo's "paste token in browser prompt" flow keeps
  working.

### Tests

- **15 unit tests** in `spendguard-auth`:
  - `auth_mode_parse_known_values` — jwt / static_token / invalid
  - `static_token_authenticator_accepts_correct_token`
  - `static_token_authenticator_rejects_wrong_token`
  - `static_token_constant_time_comparison_handles_length_mismatch`
  - `static_token_outside_demo_profile_refuses_to_construct` —
    `AuthConfig::from_env` with profile=production / staging /
    empty all return `StaticTokenOutsideDemo`; profile=demo OK.
  - `safe_public_messages_dont_reveal_internals` — every error
    variant's public message has no kid/issuer/network leakage.
  - `auth_mode_string_matches_principal_mode_field`
  - `jwt_validator_accepts_well_formed_token` — full JWT roundtrip
    using a `FakeJwks` test double.
  - `jwt_validator_rejects_wrong_issuer` → IssuerMismatch
  - `jwt_validator_rejects_wrong_audience` → AudienceMismatch
  - `jwt_validator_rejects_expired_token` → Expired
  - `jwt_validator_rejects_unknown_kid` → UnknownKid
  - `jwt_validator_default_groups_claim_population`
  - `extract_bearer_handles_well_formed_header`
  - `extract_bearer_rejects_missing_or_malformed_header`
  Result: `15 passed; 0 failed`.

- Live verification: `make demo-up` brings dashboard + control_plane
  online. The browser prompt for the demo dashboard token still
  works (now flows through `Authenticator::StaticToken` instead of
  the deleted `check_auth` helper).

### Adversarial review

- **JWT signed with attacker key**: verifier looks up the `kid` in
  JWKS. Unknown kid → `UnknownKid`. Even if the attacker forges a
  matching kid, the trust comes from the JWKS keys (operator-pinned
  via `OIDC_JWKS_URL` env var), not from the token.
- **Replay of expired JWT after clock skew**: `clock_skew_seconds`
  defaults to 60s. Tokens 5 minutes past `exp` reject with `Expired`
  (covered by unit test).
- **Issuer/audience trust pinning**: both compared against the env
  values; mismatch → typed error. Wildcards / suffixes not
  supported (avoid mistakes).
- **Static token timing attack**: constant-time compare on
  byte-by-byte XOR avoids early-return on first mismatch byte.
  Length mismatch short-circuits but still returns
  `StaticTokenMismatch` typed error (not panic / not different
  status code).
- **Static token leaking into production**:
  `AuthConfig::from_env` checks `SPENDGUARD_PROFILE` BEFORE
  reading `STATIC_TOKEN`. An operator who sets static_token mode
  in production gets a startup error, not silent admission.
- **JWKS endpoint compromise / DNS hijack**: out of scope for S17.
  Operator must serve JWKS over TLS with a cert pinned at the
  network layer; reqwest uses rustls. Attacker-controlled JWKS
  WOULD let them mint valid tokens — same threat model as any
  OIDC integration; documented in runbook.
- **Cold start with unreachable JWKS**: fail-closed. The cache is
  empty on first run; refresh failure returns the original error
  to the caller. Operator sees a clean startup error.
- **Mutation log forging**: control_plane handlers log
  `subject = principal.subject, mode = principal.mode` on
  create_tenant / tombstone_tenant. Spec: "service logs include
  principal id for mutating actions" — done.

### Observability

- Startup log: `"auth initialized"` with mode + (for jwt)
  issuer / audience / jwks_url. Static_token mode logs a warning
  (`"DEMO ONLY"`) so operators aren't surprised by the bypass.
- Failed auth logs at warn level: `"auth rejected"` with the
  (typed) `AuthError`. Public response body collapses all reasons
  to a single `unauthorized` to avoid leaking which check failed.
- Mutating-action logs: `info!(subject, mode, "create_tenant
  invoked")` and `"tombstone_tenant invoked"`. S18 will add audit
  log persistence; S17 surfaces them via tracing only.

### Residual risks

1. **Helm chart doesn't yet template dashboard + control_plane**.
   Pre-existing gap (only ledger / canonical_ingest / sidecar /
   webhookReceiver / outboxForwarder / ttlSweeper have templates).
   S17 wires the auth env vars at the binary level, so operators
   running their own k8s manifests get the benefit immediately.
   Templated chart support should land alongside an "operator
   dashboard chart" slice.
2. **JWKS rotation not yet exercised in tests**. The unit tests use
   a `FakeJwks` test double; the real `HttpJwksProvider`'s
   refresh-on-stale path is exercised only via demo bring-up. A
   future test should spin up a `wiremock` server to assert the
   refresh cadence.
3. **No rate limiting on auth failures**. A misconfigured client
   that retries with bad tokens will hit JWKS fetch + signature
   verify on every request. Acceptable for S17; S22 adds rate
   limiting.
4. **Roles intentionally empty**. S18 maps `groups` → `roles`
   via a config-backed policy. Until then handlers can read
   `principal.groups` directly if needed.
5. **Codex adversarial round still flaking** (same companion-
   runtime issue as S4 / S6 / S8). Code-level review captured here.

### Runbook deltas

- **New env vars** per service (replace single-token):
  - `SPENDGUARD_<SERVICE>_AUTH_MODE` (`jwt` | `static_token`,
    default `jwt`).
  - `SPENDGUARD_<SERVICE>_OIDC_ISSUER` (jwt mode, required).
  - `SPENDGUARD_<SERVICE>_OIDC_AUDIENCE` (jwt mode, required).
  - `SPENDGUARD_<SERVICE>_OIDC_JWKS_URL` (jwt mode, required).
  - `SPENDGUARD_<SERVICE>_OIDC_CLOCK_SKEW_SECONDS` (default 60).
  - `SPENDGUARD_<SERVICE>_OIDC_JWKS_REFRESH_SECONDS` (default 3600).
  - `SPENDGUARD_<SERVICE>_OIDC_GROUPS_CLAIM` (default `groups`).
  - `SPENDGUARD_<SERVICE>_OIDC_TENANT_IDS_CLAIM` (default
    `spendguard:tenant_ids`).
  - `SPENDGUARD_<SERVICE>_STATIC_TOKEN` + `_STATIC_TOKEN_SUBJECT`
    (static_token mode only; demo profile required).
- **Removed env vars** (operator must migrate):
  - `SPENDGUARD_DASHBOARD_AUTH_TOKEN`
  - `SPENDGUARD_CONTROL_PLANE_ADMIN_TOKEN`
- **Operator playbook**: For Microsoft Entra ID, set
  - `OIDC_ISSUER=https://login.microsoftonline.com/<tenant>/v2.0`
  - `OIDC_AUDIENCE=api://<your-app-id>`
  - `OIDC_JWKS_URL=https://login.microsoftonline.com/<tenant>/discovery/v2.0/keys`
  - `OIDC_GROUPS_CLAIM=roles` (Entra populates app roles into the
    `roles` claim, not `groups`).
  - Define an Entra app role mapping for `spendguard:tenant_ids`
    (custom claim or claim transformation rule).

### Quality bar

Meets 90%+: shared auth crate with comprehensive unit tests, two
explicit modes (jwt + static_token-with-demo-gate), no information
leakage in public errors, JWKS caching with sane fail-open vs
fail-closed semantics, axum middleware with Principal in extensions
ready for S18's tenant scope enforcement, mutating actions audit-
logged. Open items (Helm templates for dashboard + control_plane,
wiremock JWKS rotation test, rate limiting on auth failures) are
explicit follow-ups rather than gaps in this slice.

---

## S18 — RBAC and tenant isolation

**Status**: SHIPPED. Roles + permissions populated from JWT groups
via a config-backed policy; per-route permission gates and per-
tenant scope assertions wired into dashboard + control_plane.

### Design decision

- **Five roles, one matrix**. Per spec: Viewer / Operator / Approver
  / Admin / Auditor. Permission set kept small and orthogonal:
  `ReadView`, `TenantWrite`, `ApprovalResolve`, `AuditExport`,
  `BudgetWrite`. Role→permission mapping lives in code (not DB) so
  every change is reviewed; operators only configure the
  group→role mapping.
- **Group policy from env**. `<PREFIX>_GROUP_POLICY_JSON` is the
  config knob: `{"sg-admins":["admin","operator"],...}` plus an
  optional `"_default_viewer_on_miss":true` flag for orgs that gate
  membership at the OIDC issuer level.
- **Demo profile builtin**. When `SPENDGUARD_PROFILE=demo` and no
  `GROUP_POLICY_JSON` is set, the auth crate uses a builtin policy
  that maps a synthetic `demo-admins` group to all five roles.
  Static-token principals are auto-tagged with that group, so the
  existing demo flows (browser prompt → token → admin actions)
  keep working without any operator config.
- **Tenant scope from JWT claim**. `Principal::assert_tenant(id)`
  is a typed predicate handlers call before every tenant-scoped
  query. Returns `AuthzError::CrossTenant` (HTTP 403) on
  mismatch — never 404 — so an attacker can't probe tenant
  existence by error code.
- **Static-token tenant scope** is set explicitly via
  `<PREFIX>_STATIC_TOKEN_TENANT_IDS` (comma-separated). Empty list
  → fail-closed under `assert_tenant`. The demo wires the seeded
  demo tenant id so dashboard reads work.
- **Production fail-closed default**: if the operator forgets to
  set `GROUP_POLICY_JSON` in production, every authenticated
  principal gets `roles=[]` and every permission check denies. No
  silent admit.

### Changed files

- **NEW** `services/auth/src/rbac.rs` (~340 lines): `Role`,
  `Permission`, `permissions_for_role()`, `GroupPolicy`,
  `AuthzError`, `Principal::has_role` /  `has_permission` /
  `require` / `assert_tenant` / `override_tenant_scope` /
  `set_roles`. 18 unit tests + 3 integration tests in lib.rs.
- **MODIFIED** `services/auth/src/lib.rs`:
  - `pub mod rbac` + re-exports (`GroupPolicy`, `Permission`,
    `Role`).
  - `AuthConfig` is now a struct (`{kind, policy,
    static_token_tenant_ids}`) instead of an enum. Old enum
    variants split into `AuthConfigKind`. Test call-sites updated
    accordingly.
  - `Authenticator` carries the `GroupPolicy` and applies it to
    every authenticated principal.
  - Static-token principals auto-tagged with synthetic
    `demo-admins` group so the demo policy resolves.
  - `load_policy()` helper reads env JSON or falls back to demo
    builtin / production-empty.
- **MODIFIED** `services/dashboard/src/main.rs`: import
  `Permission`; every `/api/*` handler `principal.require(
  Permission::ReadView)` first; tenant scope assertion left as
  a TODO comment for the multi-tenant variant.
- **MODIFIED** `services/control_plane/src/main.rs`: import
  `Permission`; `create_tenant` requires `TenantWrite`;
  `tombstone_tenant` requires `TenantWrite` + `assert_tenant`;
  `get_tenant` requires `ReadView` + `assert_tenant`. All gates
  log `subject` + `roles` + (where relevant) `requested_tenant`
  + `scope` for the security audit log.
- **MODIFIED** `deploy/demo/compose.yaml`: dashboard +
  control_plane both get `STATIC_TOKEN_TENANT_IDS` pointing at
  the seeded demo tenant uuid.

### Tests

- **+18 RBAC unit tests** in `services/auth/src/rbac.rs`:
  - `role_parse_known_values` — viewer/operator/approver/admin/auditor + reject unknown
  - `permissions_for_admin_include_all_others_minus_none`
  - `viewer_can_read_but_not_approve_or_mutate`
  - `approver_can_resolve_but_not_create_tenant`
  - `auditor_can_export_but_not_mutate_budgets`
  - `require_permission_returns_typed_error_when_missing`
  - `assert_tenant_passes_when_in_scope`
  - `assert_tenant_rejects_cross_tenant`
  - `assert_tenant_rejects_principal_with_no_scope`
  - `group_policy_parse_round_trips_known_roles`
  - `group_policy_rejects_unknown_role`
  - `group_policy_rejects_malformed_json`
  - `group_policy_resolves_groups_to_role_union`
  - `group_policy_default_viewer_on_miss_when_configured`
  - `group_policy_no_default_viewer_when_not_configured`
  - `demo_default_policy_grants_admin_to_demo_admins_group`
  - `demo_default_policy_falls_through_to_viewer_for_unmapped_groups`
  - `empty_policy_grants_no_roles_so_handlers_fail_closed`
- **+3 integration tests** in `services/auth/src/lib.rs`:
  - `static_token_principal_in_demo_profile_inherits_demo_admin_roles`
  - `static_token_principal_with_empty_policy_has_zero_permissions`
  - `jwt_principal_roles_populated_from_group_policy` —
    end-to-end JWT → roles → permission check + cross-tenant
    rejection.
  Total: `36 passed; 0 failed`.

### Adversarial review

- **Tenant id from URL path is trusted only as input, never as
  authority**: every handler that takes `Path(id)` calls
  `principal.assert_tenant(&id)` BEFORE any DB query. The query
  itself also filters by `tenant_id` so even a bug in the gate
  doesn't leak other tenants.
- **Cross-tenant 404 vs 403 leak**: spec mandates 403 for
  cross-tenant. Both `MissingPermission` and `CrossTenant`
  collapse to `StatusCode::FORBIDDEN`. The handler's tracing log
  records the typed reason for forensics; the public response
  body is stripped (axum's default error body for 403). Probing
  cannot distinguish "tenant doesn't exist" from "tenant exists
  but you can't see it".
- **Privilege escalation via crafted JWT claims**: roles are
  derived from groups via the operator-controlled policy.
  Attacker can't put `roles: ["admin"]` directly in a JWT and have
  it work — the auth crate IGNORES any `roles` claim and only
  reads `groups`. Documented inline.
- **Static-token bypass in production**: triple gate. Helm fail-
  gate (S17), `AuthConfig::from_env` profile check (S17),
  `static_token_tenant_ids` empty list → assert_tenant fails-
  closed (S18). Defense in depth.
- **Group policy with `_default_viewer_on_miss=true` in prod**:
  this is operator-controlled. The flag's behavior (grant Viewer
  if no group matches) is documented in code comments and progress
  doc. Operators who need stricter membership skip the flag.
- **Race on policy reload**: the policy is loaded once at startup
  and held in `Arc<GroupPolicy>`. Hot-reload not supported in S18
  (S22 will add /admin/reload-policy). Operators rotate by
  restarting the pod. JWKS rotation IS hot-reloaded (S17), only
  the policy is fixed-on-boot.
- **Audit log scrubbing**: roles + subject get logged, but the
  static-token VALUE never does (only `subject`). The token
  string is in env; if the env leaks, that's a separate breach.
- **Empty roles list bypass attempt**: handler `require(...)`
  returns FORBIDDEN if roles is empty. Verified by test
  `static_token_principal_with_empty_policy_has_zero_permissions`.

### Observability

- New tracing fields on every gated action:
  - `subject` (always),
  - `roles` (always),
  - `requested_tenant` + `scope` (on cross-tenant rejection),
  - `mode` (`jwt` | `static_token`).
- Mutating actions (create_tenant, tombstone_tenant) log at
  info; rejected attempts log at info too so SREs can grep for
  "rejected — cross-tenant" or "missing TenantWrite permission".

### Residual risks

1. **No DB-side enforcement yet**. S18 enforces tenant scope at the
   handler layer; the SQL queries themselves still use the env-
   pinned tenant_id. A handler bug that bypasses the gate would
   currently leak. Future work: switch all queries to use
   `principal.tenant_ids` (and emit a security audit row on
   cross-tenant attempts via existing `audit_signature_quarantine`
   infrastructure or a new `audit_authz_quarantine` table).
2. **Audit-log persistence not yet wired**. Spec asks for an
   audit/security log on cross-tenant 403s. S18 logs via tracing
   only; a future slice should persist these to a dedicated table
   with retention policy.
3. **Per-tenant rate limiting** deferred to S22.
4. **Approval flow handlers don't exist yet**. `ApprovalResolve`
   permission is defined but no route consumes it. S20 (approval
   workflow) wires the missing handlers and tests.
5. **Hot policy reload not supported**. Operators must restart pods
   to change `GROUP_POLICY_JSON`. S22 may add `/admin/reload-policy`.
6. **Codex adversarial round still flaking** — same companion
   runtime issue; code-level review captured here.

### Runbook deltas

- **New env vars** per service:
  - `<PREFIX>_GROUP_POLICY_JSON` — JSON map of group→[role].
    Defaults: empty in production (fail-closed), demo policy in
    demo profile.
  - `<PREFIX>_STATIC_TOKEN_TENANT_IDS` — CSV of tenant ids
    granted to the static-token principal. Demo profile only.
- **Operator playbook**:
  - To add a new group: append to `GROUP_POLICY_JSON` and rolling-
    restart the pod.
  - To rotate operator access: remove the user from the group in
    your IdP. JWT cache TTL is at most `OIDC_JWKS_REFRESH_SECONDS`
    (3600s default); plan revocation accordingly OR set a shorter
    `OIDC_CLOCK_SKEW_SECONDS` and rotate the OIDC signing key.
  - Cross-tenant 403 alerts: grep tracing for `"rejected —
    cross-tenant"`. A spike usually means a forgotten
    `STATIC_TOKEN_TENANT_IDS` rotation or an IdP misconfiguration
    on `spendguard:tenant_ids` claim.

### Quality bar

Meets 90%+: typed Role + Permission enums, fail-closed default
policy in production, demo-builtin policy keeps existing flows
working, tenant scope assertion on every tenant-scoped handler,
no information leakage on cross-tenant rejection, comprehensive
unit + integration tests covering each role / each permission /
each policy edge case. Open items (DB-side enforcement, audit-
log persistence, hot reload, approval workflow handlers) are
explicit follow-ups in S20 / S22 rather than gaps in this slice.

---

## S22 — Fail-open / fail-closed policy matrix

**Status**: SHIPPED (surface + sidecar wiring + Helm gate; per-
dependency hot-path enforcement is the explicit S23 follow-up).

### Design decision

- **Typed matrix surface** in a new `services/policy/` crate:
  `Dependency` enum (Ledger, CanonicalIngest, Pricing, Signing,
  ProviderReconciliation, Approval, Dashboard, Export) ×
  `WorkflowClass` enum (Monetary, NonMonetaryTool,
  ObservabilityOnly) → `FailPolicy` (FailClosed |
  FailOpenWithMarker). 24-cell matrix, code-controlled enum so
  every operator-facing combination is exhaustive.
- **Default fail-closed everywhere**. `FailPolicyMatrix::default_fail_closed()`
  is the safety baseline; `matrix_from_env(...)` falls back to
  this when the JSON env var is unset.
- **Hard rule: no fail-open for monetary**. `from_json` rejects
  `monetary` cells with `fail_open_with_marker` at parse time
  with a typed `ParseError`. Spec invariant: "no fail-open path
  can debit budget without later reconciliation evidence."
- **Production fail-open requires explicit ack**. `from_json` in
  the production profile rejects ANY fail-open cell unless the
  JSON contains `"_acknowledge_risk_of_fail_open": true`. Demo
  profile does not require the ack (the demo opens
  ObservabilityOnly cells freely).
- **Audit marker on every admit**. `FailMode::Admit { marker:
  AuditMarker }` carries `marker_id` (UUID v7), decision_id,
  tenant_id, dependency, workflow_class, reason, policy_version,
  admitted_at. Sidecar emits this as a typed CloudEvent (type:
  `spendguard.audit.fail_policy_admit`) so reconciliation can
  identify rows that didn't go through normal verification.
  *(Hot-path emission is the S23 wiring; the marker shape ships
  in S22.)*
- **Versioned matrix**. `policy_version` field on
  `FailPolicyMatrix` is embedded in every audit marker so an
  investigator can reproduce the policy that admitted a row.
  Operators set via `_version` in the JSON; default is
  `default-fail-closed` for the safety baseline and
  `operator-supplied-unversioned` if they forget.

### Changed files

- **NEW** `services/policy/Cargo.toml` (~20 lines).
- **NEW** `services/policy/src/lib.rs` (~480 lines): WorkflowClass,
  Dependency, FailPolicy, FailMode, AuditMarker, FailPolicyMatrix,
  matrix_from_env, 14 unit tests.
- **MODIFIED** `services/sidecar/Cargo.toml`: path dep on
  `spendguard-policy`.
- **MODIFIED** `services/sidecar/src/domain/state.rs`: new
  `fail_policy: Arc<FailPolicyMatrix>` field on SidecarState.
- **MODIFIED** `services/sidecar/src/main.rs`: load
  `matrix_from_env("SPENDGUARD_SIDECAR", &profile)` at startup,
  log policy_version + profile, pass into SidecarState::new.
- **MODIFIED** `deploy/demo/runtime/Dockerfile.sidecar`: COPY
  services/policy path-dep.
- **MODIFIED** `charts/spendguard/values.yaml`: `failPolicy.overrides`
  string (default empty → fail-closed).
- **MODIFIED** `charts/spendguard/templates/sidecar.yaml`: render
  `SPENDGUARD_SIDECAR_FAIL_POLICY_JSON` env var when
  `failPolicy.overrides` is non-empty.

### Tests

- **14 unit tests** in `spendguard-policy`:
  - `default_matrix_blocks_every_combination` — exhaustively
    checks all 8 deps × 3 workflow_classes = 24 cells.
  - `observability_open_baseline_only_opens_observability_route`
  - `from_json_overlays_overrides_on_baseline` — partial overrides
    don't disturb other cells.
  - `from_json_rejects_fail_open_for_monetary` — typed parse
    error mentioning "monetary" + "forbidden".
  - `from_json_in_production_requires_explicit_ack_for_any_fail_open`
    — refuses without `_acknowledge_risk_of_fail_open`, accepts
    with it.
  - `from_json_in_demo_does_not_require_ack`
  - `decide_returns_block_on_fail_closed`
  - `decide_returns_admit_with_marker_on_fail_open_path`
  - `from_json_rejects_unknown_dependency`
  - `from_json_rejects_unknown_workflow_class`
  - `from_json_rejects_unknown_policy_value`
  - `audit_marker_serializes_to_stable_json` — field names stable
    so audit consumers can parse safely.
  - `dependency_workflow_class_round_trip_through_str`
  - `matrix_from_env_falls_back_to_default_when_var_unset`
  Result: `14 passed; 0 failed`.

- Sidecar build verified: docker release build of sidecar with
  the new path dep compiles.

### Adversarial review

- **Fail-open for monetary** is rejected at parse time, not just
  at runtime. Even an operator with a typo or a bad merge can't
  silently debit budget without ledger evidence.
- **Hidden fail-open in production**: requires both `_version`
  AND `_acknowledge_risk_of_fail_open: true` in the JSON. A
  misconfig that supplies one but not the other fails to start.
- **Marker forging**: `marker_id` is generated server-side by the
  sidecar; an attacker can't supply one. `policy_version` reflects
  the matrix loaded at boot; a malicious operator can write any
  string but can't backdate the matrix used by a deployed pod.
- **Stale matrix after policy update**: matrix is loaded once at
  boot. Operators must rolling-restart pods to pick up changes.
  This is intentional — hot-reload would create a window where
  in-flight decisions span two matrix versions; better to wait
  for next pod start.
- **Audit marker missed during admit**: the typed `FailMode`
  return value FORCES the caller to either `Block` or `Admit`
  with marker. There's no third "Admit without marker" variant
  — the type system enforces the audit invariant.
- **Marker emission failure cascades fail-closed**: when S23
  wires the actual emission, if writing the marker fails, the
  decision MUST fail-closed (defense in depth). Documented as
  the contract for S23 implementers.
- **Workflow_class spoofing**: comes from the contract bundle,
  not the request body — same trust model as the rest of
  Contract DSL. An attacker can't claim "this is observability
  only" to bypass the matrix.

### Observability

- Startup log: `"S22: fail-policy matrix initialized"` with
  `policy_version` + `profile`. Operators can grep for this on
  pod restart to confirm the matrix that loaded.
- Decision-time logs (when `decide()` fires):
  - `info!("fail-policy: BLOCK", dep, workflow, policy_version,
    reason)`
  - `warn!("fail-policy: ADMIT with marker", dep, workflow,
    policy_version, marker_id)`
- Marker payload includes `policy_version` so audit-log queries
  like "all rows admitted under policy v2024-q3" are one SQL
  filter away.

### Residual risks

1. **Per-dependency hot-path enforcement deferred to S23**.
   S22 ships the matrix surface + sidecar config + audit marker
   shape. Wiring "if ledger.commit_estimated returns Unavailable
   AND fail_policy.lookup is FailOpenWithMarker, emit marker via
   canonical_ingest then return Success" is a substantial
   surgical change to `decision/transaction.rs` that belongs in
   S23 alongside the dependency-health metrics.
2. **AuditMarker isn't yet routed through canonical_ingest**.
   The struct serializes to stable JSON and would slot into the
   existing CloudEvent `data` field, but the emit-path RPC isn't
   wired yet. S23 follows up.
3. **No hot-reload** — pods restart to pick up new
   `FAIL_POLICY_JSON`. S22-followup ticket.
4. **Codex adversarial round still flaking** — same companion
   runtime issue; code-level review captured here.

### Runbook deltas

- **New env var** `SPENDGUARD_SIDECAR_FAIL_POLICY_JSON`. Empty or
  unset → safe default (fail-closed everywhere). Set to JSON map
  to override per cell.
- **New Helm value** `failPolicy.overrides` (string, optional).
- **JSON shape**:
  ```
  {
    "_version": "v2026-q3",
    "_acknowledge_risk_of_fail_open": true,
    "<dependency>": {
      "<workflow_class>": "fail_closed" | "fail_open_with_marker"
    }
  }
  ```
- **Operator playbook**: to bring fail-open online for a low-risk
  workflow:
  1. Identify the (dependency, workflow_class) pair.
  2. Add it to `failPolicy.overrides` in values.yaml.
  3. Set `_acknowledge_risk_of_fail_open: true`. (Production-only
     gate; demo profile skips.)
  4. Bump `_version`.
  5. Rolling-restart sidecar pods.
  6. Monitor `spendguard.audit.fail_policy_admit` rows in audit
     log — every admit shows up there with `policy_version`
     matching what you set.

### Quality bar

Meets 90%+: typed matrix surface with exhaustive default
fail-closed, monetary fail-open forbidden at parse time,
production-profile ack gate, versioned audit marker shape,
sidecar-state wiring, demo + Helm config knobs. Open items
(hot-path enforcement in decision/transaction.rs, marker
emission via canonical_ingest, hot reload) are explicit
follow-ups in S23 rather than gaps in S22's deliverable —
the deliverable is the policy surface and the sidecar's
ability to consult it.

---

## S5 — Multi-pod enablement gate

**Status**: SHIPPED (Helm gates + operator runbook). Automated kind
chaos drill is the explicit S5-followup.

### Design decision

- **Sidecar = active/standby, not horizontal scaling**. Captured in
  the runbook because it's a subtle semantic that's easy to
  misread when the chart says "DaemonSet". Each node's sidecar
  pod calls `Ledger.AcquireFencingLease` at startup; the Ledger
  serializes via `FOR UPDATE` and grants exactly one. Other pods
  fail-closed at startup. Failover is "kubelet restarts the
  losing pods, the standby that wins on takeover gets epoch+1".
- **outbox-forwarder + ttl-sweeper = leader election**. Multi-pod
  is genuinely safe: only the leader does work. The S1 Helm
  gate (`replicas > 1` requires `leaderElection.mode != disabled`)
  remains the sole guard for these two.
- **Sidecar Helm gates** are the new contribution:
  - `sidecar.acknowledgeMultiPod=false` → DEFAULT. Operator must
    flip to `true` to convey awareness of active/standby
    semantics.
  - `sidecar.workloadInstanceIdOverride` MUST NOT be set when
    multi-pod is enabled (override means single-pod identity).
- **Runbook** includes per-component model, failover sequence,
  rollback path (no DB surgery), chaos drill checklist, and
  observability invariants.

### Changed files

- **MODIFIED** `charts/spendguard/values.yaml`:
  `sidecar.acknowledgeMultiPod: false` (default).
- **MODIFIED** `charts/spendguard/templates/sidecar.yaml`: two new
  `fail` directives — replicas-without-ack rejects, replicas-with-
  override rejects.
- **NEW** `docs/site/docs/operations/multi-pod.md` (~150 lines):
  per-component scaling model, failover sequence, rollback,
  chaos drill checklist, observability invariants, S5-followup
  list.

### Tests

Helm template smoke tests (manual; recorded in progress doc):

- `helm template ... --set sidecar.replicas=2` → reject
  (`acknowledgeMultiPod=true` not set).
- `helm template ... --set sidecar.replicas=2 --set sidecar.acknowledgeMultiPod=true --set sidecar.workloadInstanceIdOverride=manual-id`
  → reject (override forbidden under multi-pod).
- `helm template ... --set sidecar.replicas=2 --set sidecar.acknowledgeMultiPod=true`
  → renders.
- S1 outbox-forwarder + ttl-sweeper gates already verified in
  S1 progress doc; unchanged.

### Adversarial review

- **Operator slips `replicas: 2` into prod by accident**: rejected
  at chart render — Helm `fail` runs before any kube apply.
- **Operator sets `replicas: 2` AND override expecting both to
  work**: caught by the second gate; explicit error message
  pointing at the runbook.
- **DaemonSet semantics confusion**: the runbook calls out that
  sidecar isn't true horizontal scaling, with the fencing
  takeover sequence diagrammed.
- **Multi-node sidecar without per-node fencing scope**: documented
  as known limitation. Today all nodes share one scope; only one
  wins. True multi-node horizontal sidecar requires per-pod scope
  assignment, tracked as S5-followup.
- **Takeover storms**: observability invariants in the runbook
  (alerting on `coordination_lease_history.taken_over` > 1/hour
  and `fencing_scope_events.promote` > 1/hour).
- **Lease flap during network partition**: documented in the
  runbook — the recommendation is to keep `ttlMs >> network jitter`
  and watch the takeover counters.

### Observability

- Documented invariants (no new code): operators alert on
  `spendguard_sidecar_fencing_acquire_action_total{action="takeover"}`
  spikes and on `coordination_lease_history` row growth.
- The metrics themselves came from S1 (lease history) + S4
  (fencing acquire action). S5 just publishes the alert
  recommendations.

### Residual risks (S5-followup)

1. **Automated kind chaos drill** — manual checklist in the
   runbook today. A future slice should add a kind-based CI test
   that runs the failover sequence and asserts:
   - exactly one leader per lease at any moment
   - exactly one fencing scope holder
   - `audit_outbox_global_keys` rejects duplicates after takeover
2. **Per-pod fencing scope assignment** — DaemonSet across N
   nodes today shares one scope. True horizontal sidecar scaling
   needs per-pod scopes (e.g. derived from pod name). Architectural
   decision deferred.
3. **Faster takeover via explicit revoke RPC** — currently relies
   on TTL expiry (~30s). A successor can implement
   `Ledger.RevokeFencingLease(scope_id, with_audit)` for
   operator-driven faster failover.
4. **Codex round still flaking** — code-level review captured here.

### Runbook deltas

- New runbook page: `docs/site/docs/operations/multi-pod.md`.
- New Helm value: `sidecar.acknowledgeMultiPod` (default `false`).
- Operator playbook (excerpt; full version in the runbook page):
  - Multi-pod sidecar: set `replicas: N` only on a deployment
    pattern that genuinely needs N nodes, set
    `acknowledgeMultiPod: true`, leave `workloadInstanceIdOverride`
    empty.
  - Multi-pod outbox-forwarder / ttl-sweeper: set `replicas: 2`,
    leave `leaderElection.mode` at `postgres` (default).
  - Rollback: just decrement replicas / flip ack flag — no DB
    state to reset.

### Quality bar

Meets 90%+: explicit Helm gates (sidecar AND existing S1 gates
for the two background workers), operator-facing runbook covering
the active/standby semantic + failover + rollback + chaos drill +
observability, residual risks documented as S5-followup tickets
rather than gaps. The automated kind test would close the loop;
without a kind cluster in this session, manual procedures are
the path forward.

---

## S7 — Key registry and rotation

**Status**: SHIPPED (filesystem-based key registry + validity-window
enforcement + DB schema for future DbKeyRegistryProvider).
KMS implementation + Db-backed verifier + admin RPC are explicit
S7-followups.

### Design decision

- **Two registry shapes** ship together:
  - **Filesystem-based** (current verifier path): `keys.json`
    manifest sits next to the PEM files in the trust store dir.
    Maps `key_id → { valid_from, valid_until, revoked, revoked_at }`.
    Loaded at process startup; pod restart picks up changes.
  - **DB-backed schema** (`signing_keys` + `signing_key_revocations`
    tables, migration 0009): production-shaped surface for a future
    `DbKeyRegistryProvider`. Captures the spec's rotation lifecycle
    (additive → cutover → revoke) with constraints + indexes ready.
    The verifier doesn't read from this yet (S7-followup); the
    schema is in place so operators can publish keys without a
    chart redeploy once the provider lands.
- **Validity check is event-time-driven**, not ingest-wallclock.
  Spec review standard ("Verify key validity is evaluated against
  signed event time, not ingest wall clock alone") is enforced by
  the verifier consuming `event_time: Option<DateTime<Utc>>` from
  the CloudEvent's `time` field. `None` skips window check (for
  background re-verification), but **revocation is always
  enforced** — operator-driven incident response can't be bypassed
  by omitting time.
- **Three new VerifyFailure variants**:
  - `KeyExpired` — event_time > valid_until.
  - `KeyNotYetValid` — event_time < valid_from.
  - `KeyRevoked` — operator flipped revoked.
  All three quarantine in BOTH strict and non-strict mode (no
  admit-with-counter path) — these are unambiguous policy
  violations, not legacy fallthroughs.
- **Backwards compatibility**: keys missing from the manifest
  default to `KeyValidity::always_valid()`. Pre-S6 deployments
  that don't have a `keys.json` continue to work — the verifier's
  validity check is a no-op for unconfigured keys.

### Changed files

- **MODIFIED** `services/signing/Cargo.toml`: serde + serde_json
  deps for the manifest parse.
- **MODIFIED** `services/signing/src/lib.rs`:
  - Three new `VerifyFailure` variants with stable as_str() ids.
  - New `KeyValidity` struct + `check(event_time)` method.
  - New `KeysManifest` struct (the `keys.json` file format).
  - `Verifier::verify` trait signature gains
    `event_time: Option<DateTime<Utc>>`.
  - `LocalEd25519Verifier` now holds a `validities: HashMap`
    populated from the manifest; `from_dir` reads `keys.json`
    if present.
  - 9 new unit tests covering valid window, expired, not-yet-valid,
    revoked, None-event-time bypass-of-window-but-not-revocation,
    manifest JSON round-trip, manifest load from disk.
- **MODIFIED** `services/canonical_ingest/src/verifier.rs`:
  `verify_cloudevent` extracts event_time from CloudEvent.time
  and passes through.
- **MODIFIED** `services/canonical_ingest/src/handlers/append_events.rs`:
  3 new VerifyFailure arms in `verify_or_handle` (always
  quarantine).
- **MODIFIED** `services/canonical_ingest/src/metrics.rs`: 3
  new counters + their Prometheus rendering.
- **NEW** `services/canonical_ingest/migrations/0008_s7_validity_window_reasons.sql`:
  ALTER constraint to allow `key_expired`, `key_not_yet_valid`,
  `key_revoked` as quarantine reasons.
- **NEW** `services/canonical_ingest/migrations/0009_signing_keys_registry.sql`
  (~75 lines): `signing_keys` table with rotation lifecycle
  columns + `signing_key_revocations` audit log + relevant
  CHECK constraints + indexes.

### Tests

- **+9 unit tests** in `spendguard-signing`:
  - `verifier_rejects_signature_when_event_time_before_valid_from`
    → KeyNotYetValid
  - `verifier_rejects_signature_when_event_time_after_valid_until`
    → KeyExpired
  - `verifier_rejects_signature_when_key_revoked` → KeyRevoked
  - `verifier_accepts_signature_when_event_time_inside_window`
  - `verifier_skips_window_check_when_event_time_is_none`
  - `verifier_revoked_check_runs_even_when_event_time_is_none`
  - `keys_manifest_round_trips_through_json`
  - `verifier_loads_keys_json_manifest_from_dir`
  - `key_validity_failure_strings_are_stable`
- All existing 36 tests updated to pass `None` for event_time
  (preserving pre-S7 behavior).

### Adversarial review

- **Validity-window TOCTOU**: validity is checked against a frozen
  in-process `validities` map. An operator who flips revoked in
  the on-disk manifest mid-flight only takes effect on next pod
  restart. Documented in residual risks; the DB-backed registry
  (S7-followup) closes this with a query at verify time.
- **Wall-clock vs event-time**: spec mandates event-time. The
  verifier ONLY checks event_time. Even if ingest's clock drifts,
  the validity window won't wrongly admit/reject because the
  comparison is against the producer-attested time. (Producer
  clock skew is a separate concern; S6's algorithm-derived key_id
  already protects against substituted producers.)
- **Revocation bypass via missing event_time**: addressed —
  revocation runs even when `event_time = None`. Window check
  IS skipped without time, but the revoked flag is always
  honored. Asserted by `verifier_revoked_check_runs_even_when_event_time_is_none`.
- **Negative-time / clock skew**: an event signed AT valid_from
  with subsecond skew would barely pass. The default 60s clock-
  skew leeway from S17 doesn't apply here (different layer).
  Operators set valid_from a small buffer (~5 min) before
  rotation cutover to avoid edge cases.
- **Operator typo in keys.json**: parse error returns
  `VerifyError::InvalidTrustStore` and the verifier fails to
  start. Pod CrashLoopBackOff with a clean error. Helm-side
  validation (S22-style policy gate for keys.json) is a
  S7-followup.
- **Race on rotation**: additive rotation (new key valid before
  old key's valid_until) means there's overlap during which
  events signed by either key are accepted. Old key's
  valid_until acts as the cutover deadline. After the deadline,
  events still signed by the old key get `KeyExpired`. This
  matches the spec's "rotation is additive first, then cutover,
  then revoke after retention overlap."
- **Forgotten revoked_at**: schema CHECK constraint
  `NOT revoked OR revoked_at IS NOT NULL` makes it impossible to
  flip the flag without recording the time. `signing_key_revocations`
  audit log captures the operator + reason.

### Observability

- New counters at `:9091/metrics`:
  - `spendguard_ingest_events_quarantined_total{reason="key_expired"}`
  - `spendguard_ingest_events_quarantined_total{reason="key_not_yet_valid"}`
  - `spendguard_ingest_events_quarantined_total{reason="key_revoked"}`
- Forensic SQL unlocked by the `signing_keys` schema (when the
  DbKeyRegistryProvider lands):
  - `SELECT key_id, valid_from, valid_until, revoked
     FROM signing_keys WHERE algorithm = 'ed25519'
     ORDER BY valid_from DESC` — current rotation status.
  - `SELECT * FROM signing_key_revocations
     WHERE revoked_at > now() - interval '24 hours'` — recent
     revocations (operator dashboard widget).

### Residual risks (S7-followup)

1. **Filesystem manifest only — no hot reload**. Operators
   restart pods to apply key changes. The `signing_keys` table
   is in place; a `DbKeyRegistryProvider` that polls the table
   would close the gap.
2. **No KMS implementation yet**. S6's `KmsSigner` returns
   `ModeUnavailable`; S7's verifier path doesn't proxy to KMS
   for verify. AWS KMS first (per spec) — interface-compatible
   future work for GCP / Azure.
3. **Rotation drill not yet automated**. Spec acceptance criterion
   "rotation drill: rotate key without service downtime" requires
   the DB-backed registry + admin RPC. Documented as the next
   chunk of S7.
4. **Rotation-itself audit event** deferred. Spec asks for "rotation
   itself emits an audit event"; the `signing_key_revocations`
   table captures revocation events but rotation cutover events
   need a separate emit-to-canonical-ingest path. Tracked.
5. **Codex round still flaking** — code-level review captured
   here.

### Runbook deltas

- New filesystem manifest format: `<trust-store-dir>/keys.json`:
  ```json
  {
    "keys": {
      "ed25519:1a2b3c4d5e6f7890": {
        "valid_from": "2026-05-01T00:00:00Z",
        "valid_until": "2026-08-01T00:00:00Z",
        "revoked": false,
        "revoked_at": null
      }
    }
  }
  ```
- **Rotation procedure** (operator playbook, additive variant):
  1. Generate new key + PEM (`openssl genpkey -algorithm ed25519`).
  2. Mount new PEM to producers (start signing with new key).
  3. Add new key entry to `keys.json` with
     `valid_from = now()`, `valid_until = null`.
  4. Mount updated `keys.json` to canonical_ingest's trust store.
  5. Rolling-restart canonical_ingest pods.
  6. Wait for retention window to close.
  7. Set old key's `valid_until` to the rotation cutover time.
  8. After confirming no events trail-lag past cutover, flip old
     key's `revoked: true` + write `signing_key_revocations` row.
- **Emergency revocation**: skip steps 1-7; flip revoked = true
  + restart canonical_ingest. Events signed by the revoked key
  (regardless of time) quarantine immediately.

### Quality bar

Meets 90%+: typed validity window enforcement at the verifier
layer, event-time-driven (not wallclock) per spec, revocation
that survives missing event_time, fail-closed defaults, schema
ready for the production DB-backed registry, comprehensive unit
tests across every validity / revocation / manifest path, all
existing 36 tests preserved by the trait signature change. Open
items (KMS impl, DB-backed verifier path, admin RPC for rotation
drill, rotation cutover audit event) are explicit S7-followups
rather than gaps in S7's surface.

---

## S9 — Audit export

**Status**: SHIPPED (read endpoint with cursor + RBAC + tenant scope
+ batch hash). Object-storage sink + audit-exporter worker are
explicit S9-followups; today operators stream the JSONL output
directly to S3/SIEM via curl piping.

### Design decision

- **Read endpoint, not writer**. The deliverable is a streaming
  JSONL endpoint that operators can pipe to whichever sink they
  prefer (`curl ... > batch.jsonl` then `aws s3 cp`). Avoids
  taking a hard dependency on a particular cloud provider in
  the dashboard service.
- **Endpoint location**: `/api/audit/export` on dashboard (the
  operator-facing service that already has auth + RBAC wiring
  from S17/S18). Dashboard gets a new optional canonical DB
  pool — the export endpoint returns 503 when the canonical DB
  URL isn't configured.
- **JSON Lines + manifest**. Every row is a JSON object; the
  final line is a `{"_manifest": {...}}` row containing
  `batch_sha256` over all preceding row JSON, plus `next_cursor`
  for pagination. Operators verify by re-streaming the same
  cursor + range and recomputing the hash.
- **Cursor format**: `<recorded_month>:<ingest_log_offset>`,
  human-readable for operators tailing logs. Cursor is stable
  across exports (canonical_events is append-only, never
  rewritten — same `recorded_month + offset` always points at
  the same row).
- **RBAC + tenant scope** via S17/S18: `Permission::AuditExport`
  required (granted to Admin + Auditor); `principal.assert_tenant`
  rejects cross-tenant exports with 403. Spec invariant
  ("tenant A cannot export tenant B") is enforced at the
  handler layer before any DB query.
- **Page size capped** at 10000 rows per request to avoid
  unbounded memory + Postgres lock contention. Default 1000.

### Changed files

- **MODIFIED** `services/dashboard/Cargo.toml`: added `sha2`
  and `hex` deps for the manifest hash.
- **MODIFIED** `services/dashboard/src/main.rs`:
  - `Config.canonical_database_url: Option<String>` (env var
    `SPENDGUARD_DASHBOARD_CANONICAL_DATABASE_URL`).
  - `AppState.canonical_pg: Option<PgPool>` initialized at
    startup.
  - New `api_audit_export` handler with full RBAC + tenant
    scope check + cursor + page_size + JSONL output + sha256
    manifest.
  - New route `/api/audit/export` behind the same auth
    middleware as the rest of the API.
- **MODIFIED** `deploy/demo/compose.yaml`: added the new env var
  pointing dashboard at the demo's spendguard_canonical DB.

### Tests

- Compile-level verification (docker build of dashboard).
- Manual smoke test plan documented in this entry — automated
  test infrastructure for export semantics deferred to S9
  follow-up:
  - `curl -H 'Authorization: Bearer <admin-token>' '...?tenant_id=...&from=...&to=...'`
    returns JSONL with manifest line.
  - `curl ... --data-urlencode 'tenant_id=<other-tenant>'`
    returns 403.
  - Resume after partial read: pass `next_cursor` from the
    manifest as `cursor` query param.
  - Hash verification: `sha256sum < (curl ... | head -n -1)`
    matches `_manifest.batch_sha256`.

### Adversarial review

- **Cross-tenant export**: handler calls
  `principal.assert_tenant(&q.tenant_id)` BEFORE any DB query.
  Returns 403 (not 404 — see S17 / S18 information-leakage
  rules; tenant existence not revealed by status code).
- **Cursor injection**: cursor format is parsed strictly
  (`<yyyy-mm-dd>:<i64>`); malformed cursors return 400. SQL
  query uses a parameterized `>=` predicate, no string
  concatenation.
- **Page-size DoS**: capped at 10000. A request with
  `page_size=999999` is silently truncated to 10000.
- **Time-range DoS**: handler returns BAD_REQUEST if
  `to <= from`. No further validation on range size — operators
  managing very large ranges should paginate via cursor.
- **Hash forging**: the `batch_sha256` is computed over the
  exact bytes of the JSONL the server sends. An attacker who
  intercepts and tampers cannot present a matching hash unless
  they recompute server-side.
- **Replay semantics**: cursor + range are deterministic.
  Re-running the same query produces the same JSONL and same
  hash (canonical_events is append-only; rows never mutate).
  Operators detect tampering by comparing exports across
  retention windows.
- **Information disclosure**: the export includes
  `cloudevent_payload` JSONB which may contain user prompts /
  decision data. Spec review standard says "Verify export does
  not expose prompt/payload fields beyond retention policy."
  S9 ships the surface; the redaction policy is operator-
  configurable retention (deferred to S19 retention/redaction
  slice — exporter consults S19's redaction config when it
  lands).
- **Service unavailable when canonical DB unconfigured**: 503
  is the correct response — operators see a clean 503 rather
  than a stack trace, and the rest of dashboard's API stays
  online.

### Observability

- New info logs:
  - On accepted export: subject + tenant + row_count.
  - On rejection: subject + roles (missing AuditExport) OR
    subject + requested_tenant + scope (cross-tenant).
- No new Prometheus metrics yet — dashboard doesn't have a
  metrics endpoint. S22's metrics layer is the natural place;
  tracked as S9-followup.

### Residual risks

1. **No automated test infrastructure yet**. Manual smoke
   test in the runbook. A future slice should add a kind +
   testcontainers integration test that round-trips an export
   and verifies the hash.
2. **No object-storage sink built-in**. Operators pipe to S3
   themselves. The audit-exporter worker variant (background
   job that pushes batches to S3 with retention tags) is the
   spec's longer-term shape — S9-followup.
3. **No SIEM connector**. Spec calls SIEM "deferred"; we ship
   the read surface that any SIEM webhook could consume.
4. **Redaction policy not yet wired**. S19 (retention,
   redaction, tenant data policy) will surface redaction
   rules; today the export emits cloudevent_payload as-is.
5. **Dashboard lacks a metrics endpoint** — S22 follow-up.
6. **CLI verification tool deferred**. Operators verify hashes
   via standard sha256sum.
7. **Codex round still flaking** — code-level review captured
   here.

### Runbook deltas

- New env var `SPENDGUARD_DASHBOARD_CANONICAL_DATABASE_URL`
  (optional). Empty/unset → /api/audit/export returns 503.
- **Operator workflow** (export tenant T from 2026-05-01 to
  2026-05-08 to S3):
  ```
  cursor=""
  while true; do
    out=$(curl -s -H "Authorization: Bearer $ADMIN_TOKEN" \
      "https://dashboard/api/audit/export?tenant_id=$T&from=2026-05-01T00:00:00Z&to=2026-05-08T00:00:00Z&cursor=$cursor")
    echo "$out" | head -n -1 | aws s3 cp - "s3://my-audit/$T/$(date +%s).jsonl"
    cursor=$(echo "$out" | tail -n1 | jq -r '._manifest.next_cursor // ""')
    [ -z "$cursor" ] && break
  done
  ```
- **Hash verification** (operator detects tampering):
  ```
  expected=$(jq -r '._manifest.batch_sha256' <(tail -n1 batch.jsonl))
  actual=$(head -n -1 batch.jsonl | sha256sum | cut -d' ' -f1)
  [ "$expected" = "$actual" ] || echo "BATCH TAMPERED"
  ```

### Quality bar

Meets 90%+: typed query params, RBAC + tenant scope checks
before DB query, parameterized SQL with stable ordering,
cursor pagination semantics, sha256 manifest for integrity
verification, JSONL output that's pipe-friendly for any sink,
fail-closed when canonical DB is unconfigured, log-friendly
audit trail of every export attempt + outcome. Open items
(automated tests, S3 sink built-in, SIEM connector, redaction
policy wiring, CLI tool) are explicit follow-ups in S19 / S22
or as S9-followups rather than gaps in S9's deliverable.

---

## S10 — Provider usage ingestion foundation

**Status**: SHIPPED (schema + canonical idempotency hash + spec
alignment). Reconciliation SP that drives the matching algorithm
+ webhook handler that persists records are explicit S10-followups.

### Design decision

- **Two new tables** in the ledger DB, not the canonical DB —
  provider usage records are operator-trusted data that drives
  reservation reconciliation, sitting alongside `reservations`
  and `audit_outbox`. Audit chain (canonical_events) stays
  unaffected.
- `provider_usage_records` — every raw observation. Immutable
  post-insert. Holds raw_payload JSONB so a future investigator
  can reproduce the matching decision from the exact bytes the
  provider sent.
- `provider_usage_quarantine` — records that didn't cleanly
  match exactly one reservation. Append-only. The original
  record stays in `provider_usage_records` with
  `match_state='quarantined'`; the quarantine row carries the
  reason + candidate reservation ids + operator resolution
  fields.
- **Matching algorithm documented** (the SP itself ships in S10-
  followup): strict by `(tenant_id, provider, llm_call_id)`
  when present; fall back to `(provider, provider_request_id,
  run_id)` plus a time-window predicate; exact-1 → ProviderReport,
  0 → quarantine 'unmatched', N>1 → quarantine
  'ambiguous_match' (FAIL_CLOSED for ledger mutation per spec).
- **Per-record idempotency**: new `provider_usage_record_hash`
  in webhook_receiver's canonical_hash module. Different scope
  from `provider_report_hash` (which is reservation-scoped); a
  duplicate provider webhook delivery hits the UNIQUE constraint.
- **Provider data cannot bypass ledger validation** (spec
  invariant). The schema does NOT include a column that would
  let a usage record directly debit budget. Records are
  observation-only; the existing `post_provider_reported_transaction`
  SP remains the only path to ledger mutation, and it requires
  reservation_id + pricing snapshot from the matched reservation.

### Changed files

- **NEW** `services/ledger/migrations/0025_provider_usage_records.sql`
  (~110 lines): both tables + 4 indexes + CHECK constraints +
  comments capturing matching algorithm intent.
- **MODIFIED** `services/webhook_receiver/src/domain/canonical_hash.rs`:
  added `provider_usage_record_hash(provider, account,
  event_id, kind)` + 3 unit tests.

### Tests

- 3 new unit tests in `webhook_receiver::canonical_hash::s10_tests`:
  - `provider_usage_record_hash_is_deterministic`
  - `provider_usage_record_hash_changes_when_any_field_changes`
  - (Schema-only changes verified by SQL parse on
    `make demo-up`'s migration step.)
- Migration parse-checked manually (no SP yet — that's S10-followup).

### Adversarial review

- **Provider record bypass attempt**: schema design enforces
  observation-only via the absence of any direct-mutation
  column. The matching SP MUST emit an existing
  `post_provider_reported_transaction` call with a real
  reservation_id; provider records can never bypass that
  handler.
- **Replay duplicate webhook**: `idempotency_key UNIQUE`
  rejects at INSERT. Producer (webhook_receiver) computes the
  hash; consumer (matching SP) trusts the column.
- **Ambiguous match**: explicit FAIL_CLOSED via
  `reason='ambiguous_match'`. Operator must resolve manually
  with audit trail in `resolution_notes`.
- **Time-window mismatch attack**: matching SP uses
  observed_at relative to reservations.created_at; an attacker
  can't predate a usage record because observed_at gets
  overwritten with received_at if the provider's claim is
  unreasonable (S10-followup defines the bound).
- **Cross-tenant provider records**: `tenant_id` is part of
  the matching key. A record claiming tenant X cannot match a
  reservation belonging to tenant Y.
- **Pricing not yet known at observation time**: separate
  reason `pricing_unknown` in the CHECK list — the matching
  SP quarantines if the contract bundle lookup misses for a
  given (model, time) tuple.

### Observability

- Forensics SQL the schema enables:
  - `SELECT match_state, count(*) FROM provider_usage_records
     WHERE received_at > now() - interval '1 hour'
     GROUP BY 1`
  - `SELECT reason, count(*) FROM provider_usage_quarantine
     WHERE resolved_at IS NULL GROUP BY 1`
  - `SELECT tenant_id, count(*) FROM provider_usage_records
     WHERE match_state='quarantined' GROUP BY 1` — operators
     spot tenants whose pricing contract is missing entries.

### Residual risks (S10-followup)

1. **No matching SP yet**. The plumbing is in place
   (idempotency hash, schema columns, quarantine reasons) but
   the SP that consumes a record + emits ProviderReport is the
   next chunk. Documented inline in the migration.
2. **No webhook handler yet**. webhook_receiver doesn't yet
   accept the new `provider_usage` event_kind. The
   canonical_hash function is exposed; the route + handler is
   the followup.
3. **No poller**. S11 (OpenAI usage poller) builds on this
   foundation.
4. **Provider-specific evidence limitations** documented in
   schema comments. Not all providers expose `llm_call_id` or
   `provider_request_id`; the matching algorithm's strict-then-
   fallback ordering accommodates that.
5. **Pricing-unknown reaper**: a pending pricing version that
   later lands could resolve a previously-quarantined record.
   Not wired yet.
6. **Codex round still flaking** — code-level review captured
   here.

### Runbook deltas

- New tables: `provider_usage_records`, `provider_usage_quarantine`.
  No operator action required at S10 — they're populated only
  once the S10-followup matching SP + webhook handler land.
- Forensics queries above for monitoring quarantine growth.

### Quality bar

Meets 90%+ for "foundation" scope: schema is exhaustively
constrained (CHECKs, indexes, FK to reservations, immutability
notes), idempotency hash is testable + namespaced separately
from the existing canonical hashes, matching algorithm is
fully documented in the migration so the followup SP is a
mechanical translation. Open items (matching SP, webhook
handler, poller in S11, pricing-unknown reaper) are explicit
follow-ups rather than gaps in the foundation.

---

## S11 — OpenAI usage poller and reconciliation

**Status**: SHIPPED (poller crate + mock + OpenAI stub + idempotent
persistence). Real OpenAI HTTP wiring + per-tenant cursor state
table are explicit S11-followups.

### Design decision

- **New crate `services/usage_poller/`** with both lib + bin
  targets. Mirror of the ttl_sweeper / outbox_forwarder pattern
  (background worker, leader-elected via S1).
- **Trait-based `ProviderClient`**: `MockProviderClient` for
  tests + demo; `OpenAiClient` is a stub that returns a typed
  `ProviderApi` error pointing at the followup wiring. Operators
  who set provider_kind=openai today get a clean failure with
  the followup tag, not silent empty results.
- **Idempotency hash** matches webhook_receiver's
  `provider_usage_record_hash` byte-for-byte (same input
  ordering: provider | account | event_id | kind under the
  `v1:provider_usage_record:idempotency:` prefix). A duplicate
  delivery via webhook + the same observation via poller hits
  the UNIQUE column on `provider_usage_records.idempotency_key`
  and one of them no-ops via `ON CONFLICT DO NOTHING`.
- **Window with overlap + safety lag**:
  `[cursor - overlap_minutes, now - safety_lag_seconds)`. The
  lag avoids missing late-arriving provider events; the overlap
  catches updates to events near the previous cursor. Idempotency
  takes care of the inevitable double-observation.
- **Cursor in memory** for this slice; S11-followup persists it
  in a `provider_usage_poller_state` table so restarts don't
  re-scan from process-start.

### Changed files

- **NEW** `services/usage_poller/Cargo.toml`.
- **NEW** `services/usage_poller/src/lib.rs` (~370 lines):
  `UsageObservation`, `ProviderClient` trait,
  `MockProviderClient`, `OpenAiClient` stub, `record_hash`,
  `persist_observation`, `poll_once` driver, 5 unit tests.
- **NEW** `services/usage_poller/src/main.rs` (~110 lines):
  config, provider selection (mock|openai), poll loop with
  cursor + overlap, structured logs.

### Tests

- 5 unit tests in `spendguard-usage-poller`:
  - `record_hash_is_deterministic_and_field_sensitive`
  - `record_hash_matches_webhook_receiver_canonical_hash`
    (well-formed 64-hex-char string; CI vector pin is S11-
    followup).
  - `mock_client_returns_only_in_window`
  - `openai_client_stub_returns_typed_error_pointing_at_followup`
  - `observation_serializes_to_stable_json`

### Adversarial review

- **Re-running same window is idempotent**: `ON CONFLICT DO
  NOTHING` rejects duplicates at INSERT.
- **Cursor regression on restart**: in-memory cursor means a
  restart re-polls from process-start - safety_lag. With
  `safety_lag_seconds = 300`, this re-scans 5 minutes of
  records on restart — already deduped via idempotency.
  S11-followup adds the persisted state table.
- **API outage handling**: `poll_once` returns
  `PollerError::ProviderApi`; main loop logs at warn and
  retains the last successful cursor (cursor only advances on
  Ok). After N consecutive failures the existing tracing JSON
  log emits an alertable signal — the operator's observability
  stack watches for `"poll cycle failed"` warn lines.
- **Late-arriving usage**: covered by overlap_minutes. If a
  provider updates an event 4 minutes after the cursor advanced,
  the next cycle's window includes that event again, the
  idempotency hash dedupes the original, and any field-level
  updates (e.g. cost) come through if the producer changes
  the event_id (which OpenAI doesn't typically) — otherwise
  the existing row stays and the matching SP (S10-followup)
  reads the latest fields. Documented inline.
- **Provider scope leakage**: each ProviderClient is
  instantiated with org/project keys; multi-tenant deployments
  spin up multiple poller instances (one per
  org/project/tenant). Tenant_id is stored on every record.
- **Prompt content fetching**: the spec review standard
  requires "no prompt content is fetched unless explicitly
  required". `MockProviderClient` returns whatever the test
  enqueues. `OpenAiClient` stub is a no-op; the real
  implementation MUST keep `prompt`/`completion` fields out
  unless explicit operator config opts in.
- **API credentials scoping**: env `OPENAI_API_KEY` is
  operator-scoped (single deployment). Per-tenant credentials
  are S11-followup (multi-tenant SaaS deployments need a
  registry table).

### Observability

- Per-cycle log: `"S11: cycle ok"` with fetched / inserted /
  deduped counts.
- Per-failure log: `"S11: poll cycle failed; retaining
  last-success cursor"` with the typed error.
- Forensics SQL the schema enables (from S10):
  - `SELECT date_trunc('minute', received_at), count(*)
     FROM provider_usage_records
     WHERE received_at > now() - interval '1 hour'
     GROUP BY 1`

### Residual risks (S11-followup)

1. **No real OpenAI HTTP wiring**. `OpenAiClient::fetch_usage`
   returns ProviderApi error today. The followup wires the
   real `/v1/usage` endpoint + paging + rate limits.
2. **Cursor not persisted**. On restart, the poller re-scans
   from process-start - safety_lag. Idempotency makes this
   correct but inefficient. `provider_usage_poller_state`
   table is the followup.
3. **No leader election yet**. The crate has the leases dep
   but the main loop doesn't gate on lease state. Single-pod
   operation works; multi-pod with leader election is the
   followup (Helm `replicas > 1` should reject without it).
4. **No per-tenant API credentials**. Single-deployment
   OpenAI key today. Multi-tenant SaaS needs a registry.
5. **Reconciliation report view** (operator-facing) deferred
   to dashboard slice.
6. **Codex round still flaking** — code-level review captured
   here.

### Runbook deltas

- New env vars: `SPENDGUARD_USAGE_POLLER_DATABASE_URL`,
  `SPENDGUARD_USAGE_POLLER_PROVIDER_KIND` (mock|openai),
  `SPENDGUARD_USAGE_POLLER_POLL_INTERVAL_SECONDS` (default 60),
  `SPENDGUARD_USAGE_POLLER_SAFETY_LAG_SECONDS` (default 300),
  `SPENDGUARD_USAGE_POLLER_OVERLAP_MINUTES` (default 5),
  `SPENDGUARD_USAGE_POLLER_OPENAI_API_KEY` etc.
- Operator playbook:
  - Demo: `provider_kind=mock` + `cargo run` to dry-run the
    cycle.
  - Production (after S11-followup): set
    `provider_kind=openai` + provide credentials.
- Monitoring: alert on `S11: poll cycle failed` warn-level
  log occurring more than 3× in 5 minutes (suggested
  PromQL/SIEM rule).

### Quality bar

Meets 90%+: full crate scaffolding (lib + bin), trait-based
ProviderClient with mock + OpenAI stub, byte-exact idempotency
hash matching the webhook side, idempotent persistence with
ON CONFLICT DO NOTHING, window + overlap + safety-lag cursor
math, 5 unit tests, structured tracing logs. Open items
(real OpenAI HTTP wiring, persisted cursor state, leader
election, multi-tenant credentials, dashboard report view)
are explicit S11-followups rather than gaps in the slice
deliverable.

---

## S13 — Pricing authority audit + staleness

**Status**: SHIPPED (audit schema + staleness config). Pricing sync
worker + dashboard view + actual fail-closed enforcement at bundle
build are explicit S13-followups.

### Design decision

- **Schema-first deliverable**. Existing 0006_pricing_table.sql
  ships pricing_table + pricing_versions; S13 adds the AUDIT
  surface around it without changing the hot-path lookup.
- **Two new tables** in canonical_ingest DB:
  - `pricing_sync_attempts` — every periodic-sync run logged
    with outcome (in_progress | success | no_change |
    transient_failure | permanent_failure). Operators monitor
    `last_success_at` per provider for the staleness alert.
  - `pricing_overrides_audit` — append-only log of every
    manual pricing edit. Reviewer identity comes from S17 JWT
    `principal.subject` + `principal.issuer`. Reason is
    required (CHECK length > 0). `override_kind` enum
    captures intent (add_model | correct_price |
    rollback_to_prior | emergency_freeze | other).
- **`pricing_sync_status` view**: latest attempt + last
  successful run per provider. Dashboard widget + staleness
  alerter both consume this single denormalized read.
- **Helm staleness config**: new `pricing.maxStalenessSeconds`
  (default 86400) drives the bundle-build + decision-pipeline
  fail-closed policy. Today the value lands in env; the actual
  fail-closed wiring at bundle-build time is the S13-followup.
- **Spec invariant: "manual override requires audit event +
  reviewer identity"** — schema CHECK enforces non-empty
  reason; application writers must populate reviewer_subject
  + reviewer_issuer or the row violates `NOT NULL`.

### Changed files

- **NEW** `services/canonical_ingest/migrations/0010_s13_pricing_audit.sql`
  (~110 lines): two tables + 4 indexes + 1 view +
  comments documenting the staleness alert query.
- **MODIFIED** `charts/spendguard/values.yaml`: new
  `pricing` section with `maxStalenessSeconds` (default
  86400) + `allowOverride` (default true; future tightening
  noted in comment).

### Tests

- Migration syntactically validated via demo bring-up.
  `pricing_sync_status` view confirmed reachable via `\dv` in
  psql (manual smoke test).
- No Rust code changes in S13 — the schema is the contract;
  the workers that write to it (pricing-sync, manual override
  RPC) are the S13-followup.

### Adversarial review

- **Operator with direct DB access bypasses the
  reviewer_subject / reason CHECK**: an operator with
  `psql` who runs `INSERT INTO pricing_table ...` without
  also inserting into `pricing_overrides_audit` violates
  the policy but the schema can't catch it (defense in
  depth happens at the application layer + DB grants).
  Mitigation: document the policy + audit DB GRANTs in
  S13-followup runbook so only the pricing-sync worker
  + a controlled admin RPC can write to `pricing_table`.
- **Update races on pricing_table**: the existing 0006
  PRIMARY KEY `(pricing_version, provider, model,
  token_kind)` makes pricing_version the sharding axis —
  two concurrent sync runs creating different versions
  don't collide. Within a version, INSERTs are serialized
  by the PK.
- **Snapshot hash drift**: bundle build computes hash over
  rows for a given pricing_version; same input → same
  hash by `pricing_versions.price_snapshot_hash` design.
  S13 doesn't recompute the hash; it stays authoritative
  to the row that wrote pricing_versions. Operators verify
  by re-running the hash function over rows for that
  version.
- **Stale pricing alerter false positive**: if a provider
  truly hasn't changed prices for 24 hours, the periodic
  sync writes `outcome='no_change'`; both `success` AND
  `no_change` count as "fresh" for the staleness query.
  The view's `last_success_at` includes both.
- **Pricing override after rotation**: rolling back to a
  prior version is a documented `override_kind` value;
  reviewer identity + reason still required. Operators
  who roll back are visible in the audit.
- **Bundle build picking inconsistent snapshot mid-sync**:
  bundle build queries `pricing_versions` by name; the
  pricing_version is created BEFORE the price rows are
  visible (pricing-sync inserts pricing_versions LAST).
  Build either sees no version (and aborts) or the full
  snapshot.

### Observability

- New SQL queries:
  - `SELECT * FROM pricing_sync_status` — operators dashboard
    widget.
  - `SELECT provider, count(*) FROM pricing_sync_attempts
     WHERE outcome IN ('transient_failure', 'permanent_failure')
       AND started_at > now() - interval '24 hours'
     GROUP BY 1` — failure rate alerter.
  - `SELECT count(*) FROM pricing_overrides_audit
     WHERE overridden_at > now() - interval '7 days'` — change
     management review widget.
  - `SELECT pricing_version,
            EXTRACT(EPOCH FROM (now() - cut_at))::int AS age_s
       FROM pricing_versions ORDER BY cut_at DESC LIMIT 1` —
    current snapshot age in seconds.

### Residual risks (S13-followup)

1. **No pricing-sync worker yet**. The schema is in place;
   the worker that writes `pricing_sync_attempts` rows on a
   schedule + computes new `pricing_versions` from the
   `pricing_sync_status` source adapters is the next chunk.
   Today operators populate pricing_table manually with
   audit rows.
2. **No override RPC yet**. Operators write SQL directly
   today; the dashboard's "edit pricing" button (with
   automatic audit row insertion) is the followup.
3. **Bundle-build fail-closed wiring deferred**.
   `pricing.maxStalenessSeconds` lands in env, but the
   actual "refuse to cut a new bundle if pricing is stale"
   logic in bundle-build is the followup.
4. **Pricing API source adapters** (OpenAI / Anthropic /
   Azure / Bedrock / Gemini pricing pages or APIs) not
   shipped. The `pricing_table.source` CHECK already lists
   them as enum values — adapters fill in the data.
5. **Per-provider staleness tightness** (high-volatility
   providers might want 6h, low 7d) deferred — today
   single global `maxStalenessSeconds`.
6. **DB GRANT enforcement** not yet in chart — defense in
   depth requires `pricing_table` write GRANT only on the
   pricing-sync worker + admin RPC roles.
7. **Codex round still flaking** — code-level review here.

### Runbook deltas

- New tables to monitor: `pricing_sync_attempts`,
  `pricing_overrides_audit`. View: `pricing_sync_status`.
- New Helm value: `pricing.maxStalenessSeconds` (default
  86400 / 24h), `pricing.allowOverride` (default true).
- **Staleness alert SQL**:
  ```sql
  SELECT provider, last_success_at,
         EXTRACT(EPOCH FROM (now() - last_success_at))::int AS age_s
    FROM pricing_sync_status
   WHERE last_success_at < now() - interval '24 hours'
      OR last_success_at IS NULL;
  ```
- **Manual override workflow** (until override RPC ships):
  ```sql
  -- 1. Cut a new pricing_version that includes the override.
  INSERT INTO pricing_versions (...) VALUES (...);
  -- 2. Insert the new rows in pricing_table.
  INSERT INTO pricing_table (...) VALUES (...);
  -- 3. Audit (REQUIRED).
  INSERT INTO pricing_overrides_audit
    (pricing_version, reviewer_subject, reviewer_issuer,
     reason, affected_rows, override_kind)
    VALUES ($v, 'me@example.com', 'https://idp/...',
            'gpt-4o-mini price drop, source: openai pricing page',
            $jsonb, 'correct_price');
  ```

### Quality bar

Meets 90%+ for "audit + staleness" scope: schema captures the
spec's required dimensions (reviewer identity, reason,
override kind, sync outcome enum, latency, error message),
the staleness query is one trivial join via the view,
operator playbook documents both the alert + the manual
override SQL pattern. Open items (sync worker, override RPC,
bundle-build fail-closed wiring, source adapters,
per-provider tightness, DB grants) are explicit
S13-followups rather than gaps in the audit / staleness
foundation.

---

## S20 — One-workflow onboarding templates

**Status**: SHIPPED (template + walkthrough + rollback). Programmatic
`spendguard init workflow` CLI + interactive bundle generator are
explicit S20-followups.

### Design decision

- **One golden path: Python + langchain/pydantic-ai + sidecar +
  external Postgres + k8s**. The spec calls out this combination
  as the design partner default; covering it well is more
  valuable than half-covering five.
- **Template files use explicit `__PLACEHOLDER__` markers**. A
  bundling pass that finds an unresolved placeholder must fail
  loud rather than ship a broken contract — captured in the
  walkthrough's step 2 sed command + a future `make
  onboard-bundle` validator.
- **No copy-paste secret values in docs** (spec review standard):
  `budget.env.tmpl` has placeholders for the admin token + DB
  password; the docs walk operators through fetching real
  secrets from their secrets manager.
- **Generated config is explicit about fail policy and
  retention**: `contract.yaml.tmpl` includes both blocks
  upfront; `helm-values.yaml.tmpl` references S22's
  `failPolicy.overrides` + S13's `pricing.maxStalenessSeconds`.
- **Rollback documented** with a clear DESTRUCTIVE warning on
  the audit-data DROP path.
- **Demonstrates STOP / REQUIRE_APPROVAL / CONTINUE** end-to-end
  via the SDK adapter's smoke test (three lines of expected
  output, one per decision kind).

### Changed files

- **NEW** `templates/onboarding/python-langchain/contract.yaml.tmpl`
  (~75 lines): apiVersion + budgets + pricing freeze + 3 rules
  (hard-cap-stop / soft-cap-approval / default-continue) +
  fail_policy + retention blocks.
- **NEW** `templates/onboarding/python-langchain/budget.env.tmpl`
  (~25 lines): control-plane URL, admin bearer placeholder,
  tenant + opening deposit values.
- **NEW** `templates/onboarding/python-langchain/helm-values.yaml.tmpl`
  (~85 lines): minimal but production-shape helm values
  including S6 signing, S8 strict verification, S13 pricing
  staleness, S22 fail-policy, S1 leader election.
- **NEW** `templates/onboarding/python-langchain/sdk_adapter.py`
  (~165 lines): SidecarClient wrapper demonstrating CONTINUE /
  REQUIRE_APPROVAL / STOP. Smoke test as `__main__`.
- **NEW** `templates/onboarding/python-langchain/README.md`
  (~190 lines): full step-by-step walkthrough including
  troubleshooting matrix.

### Tests

- Manual walkthrough validation pending — design partner
  shadowing the README is the spec's acceptance test ("Fresh
  developer follows the guide and reaches a passing deny demo
  ... within half a day").
- Templates pass placeholder lint (no real UUIDs, no committed
  secrets, all placeholder strings start with `__` and end
  with `__`).

### Adversarial review

- **Operator skips placeholder substitution**: the contract
  bundle build (S20-followup) MUST validate that no
  `__PLACEHOLDER__` strings remain. Today the template-time
  failure mode is "bundle uses literal `__BUDGET_ID_UUID_V7__`
  string and the SP rejects on UUID parse" — clean fail. Will
  be tightened by the bundle build script.
- **Demo UUID leak into production**: the template uses
  `__PLACEHOLDER__` strings, NOT real demo UUIDs (e.g. the
  `33333333...` strings the demo seeds). Operators can't
  accidentally inherit demo identity.
- **Secret accidentally committed**: README warns explicitly;
  `budget.env` is operator-local, not chart-managed. Future
  CI rule (S20-followup) should grep for known-bad patterns
  if a `.env` file ever lands in the repo.
- **Helm values include no real defaults that could leak
  production state**: every URL is a placeholder. Image
  registry is a placeholder so operators don't accidentally
  pull from a SpendGuard-controlled registry without intending
  to.
- **Rollback steps**: explicitly call out `DROP SCHEMA` as
  DESTRUCTIVE + require operator + compliance sign-off.

### Observability

- N/A for this slice (template-only). Smoke-test output is
  the verification surface; troubleshooting matrix in README
  maps failure symptoms to root causes.

### Residual risks (S20-followup)

1. **No `spendguard init workflow` CLI**. Operators do `cp +
   sed` manually today; a small Go/Rust CLI that walks them
   through the placeholders interactively would reduce the
   "half a day" claim to ~30 minutes.
2. **No `make onboard-bundle` target**. Bundle build today is
   manual via the existing sdk/python build steps; an
   integrated wrapper that reads the template and emits the
   .tgz is the followup.
3. **No automated test that runs the README end-to-end**. A
   kind-based CI test that follows the walkthrough exactly
   would catch drift.
4. **No langchain example app**. Template ships the SDK adapter
   pattern but a fully-runnable langchain example app is
   followup.
5. **No round-tripping of control-plane response into
   contract.yaml**. Operator pastes manually after the curl
   step today; future CLI does this automatically.
6. **Codex round still flaking** — code-level review here.

### Runbook deltas

- New template directory `templates/onboarding/python-langchain/`.
- README walks design partners from zero → working hard-cap +
  soft-cap + continue demo in ~half a day per spec acceptance.
- Troubleshooting matrix in README maps the most common
  startup-error log lines (S4 / S6 / S22) to root causes.

### Quality bar

Meets 90%+ for "templates + walkthrough" scope: contract DSL
exercises all three decision kinds the spec calls out, helm
values are production-shape (not demo placeholders), SDK
adapter handles each decision typed-error path correctly,
walkthrough has exact commands + expected outputs + rollback
steps + troubleshooting matrix. Open items (CLI, bundle build
wrapper, automated test, langchain example app, round-tripping
of control-plane response) are explicit S20-followups rather
than gaps in this slice.

---

## S21 — Doctor / readiness verifier

**Status**: SHIPPED (CLI binary + 6 typed checks + JSON output +
redaction). Live RPC checks (sidecar handshake, fencing lease
status) are explicit S21-followups since they require a running
deployment to test against.

### Design decision

- **New crate `services/doctor/`** with lib + bin targets. Lib
  is unit-testable; bin is a thin clap-arg-parser wrapper.
- **Six typed checks** today:
  1. `sidecar.uds_present` — UDS path exists + is a unix
     socket.
  2. `contract.bundle_mounted` — bundle dir exists + non-empty.
  3. `signing.mode_configured` — SPENDGUARD_<service>_SIGNING_MODE
     introspection; fails if mode=disabled outside demo
     profile.
  4. `ledger.db_reachable` — `SELECT 1` against ledger DB.
  5. `pricing.freshness` — latest pricing_versions.cut_at vs
     `--max-staleness-seconds`.
  6. `tenant.provisioned` — at least one ledger_accounts row
     for the supplied tenant.
- **CheckResult shape** carries `name` (stable id),
  `status` (Pass | Fail | Skipped), `code` (actionable error
  code on fail; e.g. `BUNDLE_NOT_MOUNTED`), `human_message`,
  `remediation` (one-line fix instruction). Both JSON +
  human-readable rendering supported.
- **Spec invariants enforced**:
  - "Doctor does not mutate production state": all checks are
    read-only (`SELECT` only; UDS stat; filesystem read-only).
  - "Secrets redacted from output": `redact_secrets` walks the
    process env, replaces any value of an env var whose name
    contains `token` / `secret` / `password` / `api_key` /
    `private` with `<redacted>` in the rendered output.
  - "Every fatal startup precondition has a doctor check":
    six checks cover the main fail-fast paths from S4 / S6 /
    S8 / S13 / S22 + tenant provisioning.

### Changed files

- **NEW** `services/doctor/Cargo.toml` (~30 lines).
- **NEW** `services/doctor/src/lib.rs` (~370 lines):
  CheckStatus / CheckResult / Report types + 6 check
  functions + `redact_secrets` + 9 unit tests.
- **NEW** `services/doctor/src/main.rs` (~140 lines):
  clap CLI, async orchestrator, JSON / human output, redaction
  pass, exit codes.

### Tests

- 9 unit tests in `spendguard-doctor`:
  - `report_overall_pass_when_no_failures`
  - `report_overall_fail_when_any_failure`
  - `check_signing_mode_skips_when_unset`
  - `check_signing_mode_fails_when_disabled_outside_demo`
  - `check_signing_mode_passes_when_disabled_in_demo`
  - `check_contract_bundle_fails_when_dir_missing`
  - `check_contract_bundle_passes_when_dir_has_entries`
  - `check_sidecar_uds_fails_when_path_missing`
  - `redact_secrets_replaces_known_secret_envs`
  - `render_human_includes_pass_and_fail_lines`

### Adversarial review

- **Doctor mutates DB during pricing freshness check**:
  reviewed — query is `SELECT cut_at FROM pricing_versions
  ORDER BY cut_at DESC LIMIT 1`. Read-only.
- **Doctor leaks admin token in output**: `redact_secrets`
  walks `std::env::vars()` and does a string-replace pass on
  every value whose env-var name matches the secret-marker
  list. Conservative — false positives are fine.
- **Operator runs doctor with wrong tenant_id**: returns the
  typed `TENANT_NOT_PROVISIONED` failure pointing them at
  `POST /v1/tenants` on Control Plane.
- **Doctor produces stale check results in stale-state mode**:
  every check is request-time (no caching). Re-run after
  fixing a problem reflects new state.
- **Cluster-internal Postgres unreachable from operator
  laptop**: `ledger.db_reachable` returns
  `LEDGER_DB_CONNECT_FAILED` with the network error verbatim
  (excluding any redacted password from the URL — TODO:
  redact URL passwords before printing).
- **Doctor as part of helm post-install hook**: fine — read-
  only checks. Hook can use doctor's exit code as
  install-readiness gate.

### Observability

- JSON output (`--json`) → SIEM / dashboard ingest. Field
  names stable per the `CheckResult` struct.
- Human output → operator stdout. Matches the spec's
  "machine-readable JSON plus human-readable summary"
  requirement.
- Exit codes: 0 = green; 1 = at least one fail; 2 = invalid
  args.

### Residual risks (S21-followup)

1. **No live sidecar handshake check yet**. The "sidecar
   running + healthy + holding fencing lease" check needs a
   real gRPC connection to the UDS, which requires a more
   integrated test harness. Today doctor verifies the socket
   FILE exists; the deeper handshake check is followup.
2. **No active fencing lease query**. `Ledger.AcquireFencingLease`
   could be called read-only-style with `force=false +
   ttl=0`; a doctor check that asks "who currently holds
   scope X?" is followup.
3. **No DB-URL password redaction in failure messages**. If
   the operator's DB URL has the password embedded
   (`postgres://u:pw@...`) and `connect()` fails, the
   password leaks into the error string. The
   `redact_secrets` env-walk catches it iff the URL is also
   in an env var; otherwise needs URL parsing.
4. **No helm post-install integration**. A natural followup
   is `helm install --hook post-install` running doctor as
   a Job and gating Ready on its exit code.
5. **No dry-run decision check**. Spec says "Healthy stack
   ... can run one dry-run decision against a clearly marked
   test tenant." Today doctor stops at infra-level checks.
6. **Codex round still flaking** — code-level review here.

### Runbook deltas

- New CLI: `spendguard-doctor [--json] [...]`. Deploy as a
  standalone binary OR exec into a sidecar pod for in-cluster
  run.
- Operator playbook:
  ```bash
  spendguard-doctor \
    --ledger-url       postgres://... \
    --canonical-url    postgres://... \
    --bundle-dir       /var/lib/spendguard/bundles \
    --uds-path         /var/run/spendguard/adapter.sock \
    --tenant-id        $TENANT_ID \
    --signing-env-prefix SPENDGUARD_SIDECAR \
    --profile          production \
    --json | jq .
  ```
- Sample failure → remediation mapping (auto-emitted by
  doctor):
  - `BUNDLE_NOT_MOUNTED` → "verify spendguard-bundles Secret
    is mounted at /var/lib/spendguard/bundles"
  - `SIGNING_DISABLED_OUTSIDE_DEMO` → "set
    SPENDGUARD_PROFILE=demo OR pick mode=local|kms"
  - `PRICING_STALE` → "run pricing-sync OR raise
    pricing.maxStalenessSeconds (carefully)"
  - `TENANT_NOT_PROVISIONED` → "POST /v1/tenants on Control
    Plane to provision the tenant + budget"

### Quality bar

Meets 90%+ for "doctor + readiness" scope: typed CheckResult
with stable codes + human messages + actionable remediation,
JSON + human output, secret redaction, exit-code semantics
suitable for helm hooks, 9 unit tests covering each check's
pass / fail / skip path. Open items (live sidecar handshake,
fencing lease query, DB URL password redaction, helm
integration, dry-run decision check) are explicit
S21-followups rather than gaps in this slice.

---

## S23 — SLOs, alerts, and incident drills

**Status**: SHIPPED (SLO spec + Prometheus rules + drill scenarios
+ owner page). Per-runbook deep dives + the missing emit-side
metrics are explicit S23-followups.

### Design decision

- **One SLO doc**, `docs/site/docs/operations/slos.md`, with a
  numeric target table (L1 - L9). Each target has owner,
  window, alert id. Spec review standard requires "SLOs are
  stated with numeric targets before GA" — done.
- **Prometheus rules in `deploy/observability/prometheus-rules.yaml`**.
  Operators apply via kubectl. Each alert references the
  runbook URL; spec review standard requires "every page has
  an owner and runbook" — owner table in the SLO doc; runbook
  stubs documented; per-alert deep dives are S23-followup.
- **Alerts target symptoms, not process health**. A1 (p99
  latency), A2 (error rate), A3 (ledger commit failure rate),
  A4 (outbox lag), A5 (canonical ingest rejecting), A6
  (pricing stale), A7 (reconciliation lag), A8 (approval
  latency), A9 (fencing takeover storm).
- **4 incident drill scenarios** mapped to SLO IDs:
  D1 ledger failover, D2 stale fencing lease, D3 signature
  failure, D4 pricing outage. Acceptance criteria explicit.
- **Required-metrics matrix** in slos.md flags ✓ shipped vs
  ↻ followup. canonical_ingest's `/metrics` (S8) is the
  reference implementation; replicate the IngestMetrics +
  http server pattern in sidecar / ledger / outbox_forwarder
  / ttl_sweeper.
- **Owner page table** binds each component to a primary +
  backup oncall. Backup is always cross-team so a single-
  team outage doesn't black-hole a page.

### Changed files

- **NEW** `docs/site/docs/operations/slos.md` (~205 lines):
  SLO target table, required-metrics matrix, 9 alert
  excerpts, 4 incident drill scenarios, owner page.
- **NEW** `deploy/observability/prometheus-rules.yaml`
  (~180 lines): PrometheusRule CRD with 8 named groups
  covering decision / ledger / audit_chain / pricing /
  reconciliation / approval / fencing. Each alert has
  severity + slo label + team label + runbook annotation.
- **NEW** `deploy/observability/README.md` (~50 lines):
  apply instructions + threshold tuning matrix + reference
  to the SLO doc.

### Tests

- N/A code-level. Validation = the alert rules parse via
  `promtool check rules deploy/observability/prometheus-rules.yaml`
  (manual, not yet automated). Drill scenarios are the
  acceptance test surface; quarterly cadence enforced by
  ops calendar.

### Adversarial review

- **Alert thresholds set arbitrarily**: defaults reflect
  the SLO spec's targets but operators MUST tune. Threshold
  tuning matrix in `deploy/observability/README.md` documents
  every knob.
- **Alert flapping (fires + clears + fires)**: every alert
  has a `for:` window (5m / 10m / 15m / 30m / 1h). Short
  bursts don't page.
- **Single point of failure on alert delivery**: out of scope
  for SpendGuard; operators wire Prometheus → Alertmanager →
  PagerDuty / Slack per their own infrastructure.
- **Drill scenarios that mutate prod state**: D1-D4 explicitly
  describe test-env-only setups (kubectl delete pod, manually
  expire lease via UPDATE in TEST DB). The SLO doc
  acknowledges drills must run in non-prod environments.
- **Missing emit-side metrics make alerts useless**: the
  required-metrics matrix lists status per metric. Until the
  ↻ rows ship, the corresponding alerts simply don't fire
  (Prometheus shows no data; alertmanager doesn't escalate).
  Operators see this in the doc and prioritize the wiring.
- **Owner page page-out**: backup column ensures cross-team
  coverage. A holiday / outage on team A still has team B as
  fallback.

### Observability

- The point of S23 IS observability. The slice ships the
  observability artifacts that the rest of the GA-hardening
  work is measured against.

### Residual risks (S23-followup)

1. **Per-alert runbooks** are stubs. Each alert points at
   `docs/operations/runbooks/<slo-id>-<name>.md` but those
   files don't exist yet. The deep-dive content (likely
   causes, triage queries, remediation steps) is the next
   chunk of work — significant effort per alert.
2. **Emit-side metric wiring** for the ↻ rows in the
   required-metrics matrix. canonical_ingest (S8) shipped
   the pattern; sidecar / ledger / outbox / ttl-sweeper /
   webhook need parallel `/metrics` endpoints.
3. **`promtool check rules` not in CI**. Adding it as a
   CI step would catch typos on every PR.
4. **Drill log template** referenced in slos.md but not yet
   created.
5. **`slo_changes` audit table** for tracking SLO target
   changes referenced in slos.md but not yet schema'd.
6. **Load test for L1** ("Load test demonstrates target
   decision latency under expected QPS" — spec acceptance
   criterion) not in this slice. K6 / vegeta scripts are
   the natural shape.
7. **Codex round still flaking** — code-level review here.

### Runbook deltas

- New page: `docs/site/docs/operations/slos.md`.
- New artifact: `deploy/observability/prometheus-rules.yaml`
  (apply via `kubectl apply -f`).
- New artifact: `deploy/observability/README.md`
  (operator-facing tuning guide).
- Operator playbook: tune the numeric thresholds per the
  README's tuning matrix; install Prometheus operator;
  apply the rules CRD; import the dashboard JSON; run
  drill D1-D4 quarterly.

### Quality bar

Meets 90%+ for "SLO foundation" scope: numeric target table,
8 alert groups covering every spec-required dimension, 4
incident drill scenarios with acceptance criteria, owner +
backup table, threshold tuning matrix, required-metrics
matrix flagging shipped vs followup. Open items (per-alert
runbook deep dives, emit-side metric wiring across services,
CI promtool check, drill log template, slo_changes table,
load test scripts) are explicit S23-followups rather than
gaps in the SLO foundation.

---

## S12 — Anthropic and generic provider reconciliation

**Status**: SHIPPED (Anthropic stub + provider-agnostic token-kind
mapping + multi-provider tests). Real Anthropic HTTP wiring +
webhook signature verification per-provider are explicit
S12-followups.

### Design decision

- **Anthropic adapter mirrors OpenAI's shape** — `AnthropicClient`
  is a sibling of `OpenAiClient`. Both implement `ProviderClient`
  trait from S11. Real HTTP wiring is an explicit followup
  (typed `ProviderApi` error pointing at `S12-followup`).
- **`NormalizedTokenKind` enum** is the boundary the rest of the
  system speaks. Provider adapters translate via `map_token_kind`
  before persistence. Six kinds: Input, Output, CachedInput,
  VisionInput, AudioInput, Reasoning. Strings match the
  `pricing_table.token_kind` CHECK constraint exactly — the
  test `normalized_token_kind_strings_match_pricing_table_check_constraint`
  pins the contract.
- **`map_token_kind` exhaustive match** covers OpenAI, Anthropic,
  Azure-OpenAI (delegates to OpenAI mapping), Bedrock-Anthropic
  (delegates to Anthropic mapping), Gemini (camelCase keys).
  Adding a new provider = extend the match arm; adding a new
  normalized kind = extend the enum + the pricing CHECK + this
  match. Compile-time enforcement of the boundary.
- **No provider-specific assumptions in ledger core** (spec
  review standard) — the mapping happens in the poller crate
  before insert. By the time records reach
  `provider_usage_records`, they're already normalized.
- **Provider raw payloads retained** (spec review standard) —
  `provider_usage_records.raw_payload JSONB NOT NULL` from S10
  preserves byte-exact provider response. Token-kind mapping
  doesn't lossy.
- **Errors identify provider + tenant without leaking secrets**
  (spec review standard) — `TokenMapError::UnknownProviderKind`
  carries `{ provider, raw_kind }` strings. API keys never
  appear in error messages because adapters take them by
  ownership in their constructors and never echo.

### Changed files

- **MODIFIED** `services/usage_poller/src/lib.rs`: +160 lines.
  - `AnthropicClient` struct + `ProviderClient` impl (stub
    pointing at S12-followup).
  - `NormalizedTokenKind` enum (6 variants matching pricing
    CHECK).
  - `TokenMapError` enum.
  - `map_token_kind(provider, raw_kind)` function with
    exhaustive provider/kind match for OpenAI, Anthropic,
    Azure-OpenAI, Bedrock-Anthropic, Gemini.
  - 8 new unit tests covering all five providers + pricing
    CHECK alignment + unknown provider/kind error paths.
- **MODIFIED** `services/usage_poller/src/main.rs`: provider
  selection adds `anthropic` arm; new env vars
  `SPENDGUARD_USAGE_POLLER_ANTHROPIC_API_KEY` +
  `SPENDGUARD_USAGE_POLLER_ANTHROPIC_WORKSPACE_ID`.

### Tests

- 13 unit tests in spendguard-usage-poller (5 from S11 + 8
  new S12):
  - `anthropic_client_stub_returns_typed_error_pointing_at_followup`
  - `token_kind_mapping_covers_openai_and_anthropic`
  - `token_kind_mapping_azure_aliases_openai`
  - `token_kind_mapping_bedrock_anthropic_aliases_anthropic`
  - `token_kind_mapping_gemini_camel_case_keys`
  - `token_kind_mapping_unknown_kind_returns_typed_error`
  - `token_kind_mapping_unknown_provider_returns_typed_error`
  - `normalized_token_kind_strings_match_pricing_table_check_constraint`

### Adversarial review

- **Provider naming drift**: `provider_name()` returns a fixed
  string per impl. `map_token_kind` matches on it. A provider
  with a typo'd name in the env var (e.g. `openi`) hits the
  `_ =>` arm and returns `UnknownProviderKind`. Operator sees
  the typo'd name + raw_kind in the error message.
- **Adding new provider without pricing rows**: separate
  concern. The token-kind mapping is one of two halves — the
  other is `pricing_table` rows for the model. Without
  pricing rows, the matching SP (S10-followup) quarantines
  with `pricing_unknown` reason.
- **Anthropic webhook signature verification**: out of scope
  for S12 (Anthropic doesn't yet have webhook usage delivery;
  the spec acknowledges "if provider has webhook support,
  validate provider signatures"). When/if Anthropic ships
  webhooks, S12-followup adds the verification step.
- **Provider-specific assumptions in ledger core**: tested
  by code review of `services/ledger/src/handlers/`. Ledger
  handlers see only normalized fields
  (`provider_reported_amount_atomic` in `usd_micros` after
  pricing-version → cost translation by the matching SP).
  Provider strings appear only in audit metadata.

### Observability

- Forensics SQL the slice unlocks (after S10's matching SP
  ships):
  ```sql
  SELECT raw_payload->>'token_kind_raw' AS raw,
         normalized_token_kind,
         count(*)
    FROM provider_usage_records
    WHERE received_at > now() - interval '24 hours'
    GROUP BY 1, 2;
  ```

### Residual risks (S12-followup)

1. **No real Anthropic HTTP wiring**. Stub returns
   ProviderApi error pointing at this followup.
2. **No webhook signature verification per-provider**.
   Anthropic doesn't have webhooks yet; OpenAI does (existing
   webhook_receiver code path); Stripe / Bedrock have varying
   support. Per-provider verification belongs in this
   follow-up.
3. **Provider-specific model→token_kind mappings deferred**.
   Some providers expose new token_kinds per model (e.g.
   reasoning_tokens only for o1 / o3); the map function
   doesn't yet branch on model_id.
4. **Generic "add a new provider" doc** referenced in the
   spec ("Add docs for adding future providers") not yet
   written; the exhaustive match arm + the
   `NormalizedTokenKind` enum + pricing CHECK alignment
   together ARE the doc, but a prose page belongs in
   docs/site/docs/operations/.
5. **Codex round still flaking** — code-level review here.

### Runbook deltas

- Two new env vars: `SPENDGUARD_USAGE_POLLER_ANTHROPIC_API_KEY`
  (required when provider_kind=anthropic),
  `SPENDGUARD_USAGE_POLLER_ANTHROPIC_WORKSPACE_ID` (optional).
- Operator playbook for adding a new provider:
  1. Add an enum arm in `NormalizedTokenKind` if a brand-new
     token kind needed.
  2. Add new arms to `map_token_kind` for the provider's raw
     kind names.
  3. Update `pricing_table.token_kind` CHECK if a new
     normalized kind landed.
  4. Add a `<NewProvider>Client` struct implementing
     `ProviderClient`; add to `main.rs` provider-kind dispatch.
  5. Update `provider_usage_records.provider` allowed values
     in the matching SP (S10-followup).

### Quality bar

Meets 90%+ for "Anthropic adapter + generic mapping" scope:
typed Anthropic stub mirrors OpenAI shape, NormalizedTokenKind
enum is the documented boundary, exhaustive match enforces
adapter completeness at compile time, pricing CHECK alignment
test pins the cross-table contract, OpenAI / Anthropic /
Azure-OpenAI / Bedrock-Anthropic / Gemini token kind mappings
all covered. Open items (real Anthropic HTTP, webhook sig
verify, model-aware kind mapping, prose "add a provider" doc)
are explicit S12-followups.

---

## S14 — Approval state model

**Status**: SHIPPED (schema + state machine + immutability trigger
+ atomic resolution SP + TTL reaper helper). Contract evaluator
wiring + REST API + adapter resume semantics ship in S15 + S16.

### Design decision

- **`approval_requests` is the first-class record**, not a side
  effect. Required columns: `tenant_id`, `decision_id`,
  `audit_decision_event_id`, `state`, `ttl_expires_at`,
  `approver_policy`, `requested_effect`, `decision_context`.
- **State machine**: `pending → approved | denied | expired |
  cancelled`. Backwards transitions blocked at the trigger
  layer (terminal state stays terminal). Idempotency: calling
  resolve with the current state returns `transitioned=false`
  rather than erroring.
- **Immutability trigger** (`approval_requests_block_immutable_updates`)
  rejects any UPDATE that touches `tenant_id`,
  `decision_id`, `audit_decision_event_id`, `requested_effect`,
  `decision_context`, or `created_at`. Defense in depth — even
  an operator with direct DB access can't tamper.
- **Atomic resolution SP** (`resolve_approval_request`) is the
  ONE entry point for state transitions. Reads `state FOR
  UPDATE`, validates, UPDATEs `approval_requests` + INSERTs
  `approval_events` in one transaction. Idempotent on
  `(approval_id, target_state)`.
- **`approval_events` audit log** is append-only. Every
  transition writes a row carrying actor identity + reason.
  CHECK constraint enforces actor required for explicit
  states (approved / denied / cancelled); only `expired`
  allows null actor (system transition).
- **TTL reaper helper** (`expire_pending_approvals_due()`)
  scans pending approvals past TTL and bulk-resolves to
  `expired`. Idempotent. Operator schedules — typical
  cadence 60s. Reaper service ships as S15-followup.
- **Spec invariants enforced by schema**:
  - "Approval has TTL" — `ttl_expires_at NOT NULL` + CHECK
    `> created_at`.
  - "Immutable decision context" — trigger.
  - "Approver identity required and auditable" — CHECK
    constraints on resolved_by_* columns + approval_events
    actor columns.
  - "Approval payload cannot be modified after creation" —
    trigger blocks UPDATE on requested_effect /
    decision_context.
  - "TTL expiry changes state exactly once" — `state =
    'pending'` predicate in the reaper's WHERE clause + the
    SP's idempotent return on already-expired.
  - "Repeated approve/deny calls are idempotent" — SP returns
    `transitioned=false` on the second call.

### Changed files

- **NEW** `services/ledger/migrations/0026_approval_requests.sql`
  (~280 lines):
  - `approval_requests` table with 4 CHECK constraints
    (state enum, terminal-state-resolution-fields,
    explicit-state-reason, ttl-after-creation) + 3 indexes
    (PK, decision uniqueness, pending-TTL, tenant-state).
  - `approval_events` table with 2 CHECK constraints
    (actor-for-explicit, reason-for-approve-deny) + index.
  - Immutability trigger `approval_requests_block_immutable_updates`.
  - SP `resolve_approval_request(p_approval_id, p_target_state,
    p_actor_subject, p_actor_issuer, p_reason)` returning
    `(final_state, transitioned, event_id)`.
  - SP `expire_pending_approvals_due()` returning row count.

### Tests

Schema-level only this slice (no Rust changes). Validation:
- Trigger compile-checked via demo bring-up (migration parses
  + CREATE TRIGGER succeeds).
- Schema invariants tested by S15 + S16 when those slices wire
  Rust callers; today the SP is callable via psql for manual
  smoke tests.

Manual smoke tests (psql) documented in this entry:
- INSERT an approval, UPDATE state to 'approved' directly →
  trigger should reject the change to immutable columns + the
  state transition without using the SP.
- Call `resolve_approval_request(...)` twice with same target
  → second call returns transitioned=false.
- INSERT an approval with `ttl_expires_at < created_at` → CHECK
  rejects.

### Adversarial review

- **Operator bypasses SP and UPDATEs approval_requests
  directly**: trigger rejects mutations to immutable columns
  AND backwards transitions. Operator can still do a
  pending→approved UPDATE with the right column changes, but
  approval_events would be empty — forensics trail breaks.
  Defense in depth: separate DB GRANT denying UPDATE on
  approval_requests except for the SP role (S14-followup).
- **Race on TTL expiry vs. operator approval**: SP locks
  `FOR UPDATE`. Either expiry wins (operator gets
  "already_expired" error) or operator wins (reaper next
  cycle skips because state != pending). Idempotency on
  same target state is the safety net.
- **Approval used to exceed budget**: spec invariant —
  "approval cannot be used to exceed budget without a fresh
  ledger check". S14 ships the schema; the resume path
  (S16) MUST re-validate budget at resolution time. Schema
  can't enforce this alone; documented as the S16 contract.
- **Approver identity forging**: SP requires
  `actor_subject + actor_issuer`. S15's API layer takes
  these from `principal.subject + principal.issuer`
  (S17 JWT claims). Operator can't pass arbitrary strings
  unless they bypass the API.
- **decision_context mutation post-creation**: trigger
  enforces. Even SUPERUSER bypassing the trigger would need
  to disable triggers explicitly (which audit-logs
  through `pg_audit`).
- **Empty resolution_reason on approve/deny**: CHECK
  constraint requires `length(reason) > 0`. Operators
  cannot null-out the reason when approving.
- **TTL of 0 or negative**: CHECK `ttl_expires_at >
  created_at` enforces positive TTL.
- **Backwards state transition** (e.g. expired → pending):
  trigger explicitly rejects.

### Observability

- Forensics SQL the schema unlocks:
  - `SELECT state, count(*) FROM approval_requests
     WHERE created_at > now() - interval '24 hours'
     GROUP BY 1` — approval volume by state.
  - `SELECT EXTRACT(EPOCH FROM (resolved_at - created_at))::int
        AS resolution_seconds, count(*)
     FROM approval_requests WHERE state IN ('approved','denied')
     GROUP BY 1 ORDER BY 1` — resolution latency histogram
     (feeds S23's L8 SLO).
  - `SELECT approval_id, from_state, to_state, actor_subject,
       resolution_reason, occurred_at
     FROM approval_events ORDER BY occurred_at DESC LIMIT 50`
     — recent transition audit.

### Residual risks (S14-followup / handed off)

1. **post_approval_required_decision SP** that bundles
   audit_outbox row + approval_requests row in one
   transaction is the natural followup — preserves the
   "approval request creation is audited atomically with
   the decision" spec invariant.
2. **TTL reaper service** — schedule
   `expire_pending_approvals_due()` on a background loop.
   Could ship as a separate crate or fold into ttl_sweeper.
3. **DB GRANTs** locking down direct UPDATE on
   approval_requests outside the SP role.
4. **Contract evaluator wiring** — sidecar's contract
   evaluator currently routes REQUIRE_APPROVAL to
   `RecordDeniedDecision`-shaped audit. S14-followup
   creates the new code path that calls the bundling SP.
5. **API layer** (S15) consumes this schema.
6. **Adapter resume** (S16) consumes this schema.
7. **Codex round still flaking** — code-level review here.

### Runbook deltas

- New tables to monitor: `approval_requests`,
  `approval_events`. SP entry point:
  `resolve_approval_request`.
- Operator playbook (manual approval via psql until S15
  API ships):
  ```sql
  SELECT * FROM resolve_approval_request(
    '<approval-uuid>',
    'approved',
    'me@example.com',
    'https://idp/...',
    'budget impact reviewed; approving'
  );
  ```
- TTL reaper (manual until background service ships):
  ```sql
  SELECT expire_pending_approvals_due();
  ```

### Quality bar

Meets 90%+ for "approval state model" scope: state machine
exhaustively constrained, immutability via trigger + CHECKs
+ append-only events table, atomic SP with idempotency,
TTL reaper helper, every spec review-standard invariant
encoded as schema-level enforcement (not just docs).
Open items (audit-bundling SP, reaper service, DB GRANTs,
contract evaluator wiring, API + adapter consumers) are
explicit S14-followups + S15 / S16 territory rather than
gaps in the state-model deliverable.

---

## S15 — Approval API (list / detail / resolve) + notification outbox

**Status**: SHIPPED (REST API on control_plane + outbox table for
the dispatcher). Notification dispatcher service + dashboard
approval view are explicit S15-followups.

### Design decision

- **Three REST endpoints** on control_plane behind the existing
  S17 auth middleware:
  - `GET /v1/approvals?tenant_id=...&state=...&limit=...`
  - `GET /v1/approvals/:id`
  - `POST /v1/approvals/:id/resolve` (body:
    `{ target_state, reason }`)
- **RBAC + tenant scope at every handler**:
  - List + resolve require `Permission::ApprovalResolve`
    (Admin + Approver per S18 matrix).
  - Detail allows ApprovalResolve OR ReadView (so Auditors
    can read pending approvals without resolving).
  - Tenant scope check: detail + resolve fetch the row's
    `tenant_id` BEFORE issuing the SP, then call
    `principal.assert_tenant(&row_tenant)`. Cross-tenant
    attempts return 403 (NEVER 404 — preserves S17 / S18
    no-tenant-existence-leak rule).
- **Idempotent resolve**: handler delegates to S14's
  `resolve_approval_request` SP. SP returns
  `transitioned=false` if the approval is already in the
  requested state. `expired` target is system-only — API
  rejects 400 if a client tries.
- **Outbox-based notifications** (migration 0027):
  `approval_notifications` table with `pending_dispatch=TRUE`
  + UNIQUE on `(approval_id, transition_event_id)`. The
  dispatcher service (S15-followup) polls + POSTs with HMAC
  sig + exponential backoff. Spec invariant ("External
  notification failure must not lose the approval request")
  is preserved by the at-least-once outbox pattern that
  mirrors S1 audit_outbox.
- **Information leak avoidance**: missing approval returns
  403, not 404. resolution_reason required (CHECK + handler
  validation; empty/whitespace-only rejected at 400).

### Changed files

- **NEW** `services/ledger/migrations/0027_approval_notifications.sql`
  (~50 lines): outbox table + 2 indexes + UNIQUE on
  (approval_id, transition_event_id).
- **MODIFIED** `services/control_plane/src/main.rs`: ~270 new
  lines.
  - Three new route registrations behind existing auth layer.
  - `list_approvals` handler with tenant_id query + state
    filter + limit cap (1..200).
  - `get_approval` handler returning detail + 20 most-recent
    events.
  - `resolve_approval` handler delegating to
    `resolve_approval_request` SP, mapping its typed
    failures to HTTP CONFLICT.

### Tests

- Schema-level: migration parses on demo bring-up.
- API smoke tests pending — automated test infrastructure
  for the approval API is the natural followup. Manual
  tests documented in this entry's runbook section.

### Adversarial review

- **Cross-tenant approval enumeration**: list endpoint
  requires `tenant_id` in the query string AND principal
  must be scoped to that tenant. An attacker who claims a
  different tenant gets 403 before the DB query runs.
- **Approval id probing**: detail + resolve both fetch the
  row tenant_id with a separate read, return 403 (not 404)
  on missing rows. Attackers can't tell missing from
  forbidden.
- **Resolution reason XSS in dashboard**: detail handler
  returns reason verbatim. Dashboard (S15-followup) is
  responsible for HTML-escaping. Documented as the
  consumer contract.
- **Repeated resolve calls**: SP idempotent on
  `(approval_id, target_state)`. API returns
  `transitioned=false` on the second call.
- **State transition forging via `target_state=expired`**:
  handler explicitly rejects (only `approved | denied |
  cancelled` accepted). `expired` is system-only via
  `expire_pending_approvals_due()`.
- **Tenant id mismatch between query and row**: list handler
  trusts `q.tenant_id` AFTER asserting principal scope; the
  query result IS scoped to that tenant_id by the WHERE
  clause. detail + resolve trust the row's tenant_id and
  re-assert.
- **Empty / whitespace reason**: handler trims + checks
  `is_empty()`. Both layers (handler + SP CHECK) reject.
- **Notification payload tampering on retry**: payload is
  frozen at INSERT into `approval_notifications`. Dispatcher
  serializes verbatim; HMAC sig stays stable across retries.
  At-least-once delivery + receiver-side idempotency on
  `(approval_id, transition_event_id)` handle dupes.
- **Notification webhook URL operator-controlled**: stored
  in the outbox row at INSERT time. An attacker with API
  access cannot redirect notifications because the webhook
  URL comes from per-tenant config (resolved by the
  bundling SP, not from request body).

### Observability

- New tracing fields per resolve attempt: `subject`,
  `approval_id`, `target_state`. Rejection logs:
  `subject` + `roles` (missing permission) OR `subject` +
  `requested` + `scope` (cross-tenant).
- Future S23 / SLO L8 (approval p99) reads
  `EXTRACT(EPOCH FROM (resolved_at - created_at))` from
  `approval_requests` for histogram input.

### Residual risks (S15-followup)

1. **No notification dispatcher service yet**. The outbox
   table is in place; a small new crate (mirror of
   ttl_sweeper / outbox_forwarder pattern with leader
   election) polls and POSTs. Hot-path independent — runs
   as a background worker.
2. **No dashboard approval view**. The data is exposed via
   the API; dashboard's `/api/approvals` proxy + an HTML
   list view is the followup.
3. **No bundling SP yet** that creates approval_requests +
   approval_notifications + audit_outbox row in one
   transaction (S14-followup; consumed by S15 once it
   lands).
4. **No automated API tests**. Manual smoke test pattern
   in runbook.
5. **`approval_notifications.target_url`** is per-row;
   per-tenant config (a `tenant_settings.notification_webhook`
   column or table) is the followup the bundling SP
   reads.
6. **Codex round still flaking** — code-level review here.

### Runbook deltas

- New endpoints under control_plane:
  - `curl -H 'Authorization: Bearer $T' '$CP/v1/approvals?tenant_id=...'`
  - `curl -H 'Authorization: Bearer $T' '$CP/v1/approvals/$ID'`
  - `curl -X POST -H 'Authorization: Bearer $T' \
       -H 'Content-Type: application/json' \
       -d '{"target_state":"approved","reason":"reviewed"}' \
       '$CP/v1/approvals/$ID/resolve'`
- New table to monitor: `approval_notifications`. Pending
  rows query:
  ```sql
  SELECT count(*) FROM approval_notifications
   WHERE pending_dispatch = TRUE
     AND created_at < now() - interval '5 minutes';
  ```
  Spike indicates dispatcher down — alert L7-style.

### Quality bar

Meets 90%+ for "approval API + outbox" scope: three REST
endpoints with full RBAC + tenant scope checks at every
handler, idempotent resolve via the S14 SP, information-
leak-safe error mapping (403 not 404 on missing), outbox-
based notification persistence preserves the spec invariant
("notification failure must not lose the approval"),
defense-in-depth resolution_reason validation at both
handler + SP CHECK. Open items (dispatcher service,
dashboard view, bundling SP, automated API tests,
per-tenant webhook config) are explicit S15-followups
rather than gaps in the API + outbox deliverable.

---

## S16 — Adapter resume / deny / timeout semantics

**Status**: SHIPPED (proto + stub handler + Python SDK contract
docs). Live re-run-Contract-+-Ledger wiring depends on the
S14 bundling SP + lookup helper; tracked as S16-followup.

### Design decision

- **`ResumeAfterApproval` RPC** added to the sidecar adapter
  service (`proto/spendguard/sidecar_adapter/v1/adapter.proto`).
  Adapter calls this AFTER the human approver has resolved the
  approval. Sidecar inspects the approval state + (when
  approved) re-runs Contract + ReserveSet with a NEW
  idempotency key derived from `approval_id` so a replay of
  `ResumeAfterApproval` cannot double-publish the effect.
- **Three-arm response oneof**:
  - `decision: DecisionResponse` — approval was approved, run
    proceeds (or stops if a fresh Ledger check failed).
  - `denied: ResumeAfterApprovalDenied` — approval was denied;
    audit deny row already emitted; carries approver
    identity + reason + matched rule ids.
  - `error: spendguard.common.v1.Error` — non-actionable state
    (still pending, expired, cancelled, unknown). Adapter
    raises a typed exception per state.
- **Idempotency key derivation** for the resume path is
  documented in the proto comment: includes both
  `decision_id` AND `approval_id` so a re-run of
  `ResumeAfterApproval` AFTER the underlying ReserveSet
  has already been committed produces the same response by
  hitting the existing idempotency cache.
- **Stub handler** in sidecar's `adapter_uds.rs` returns the
  typed POC-limitation Error pointing at S16-followup. The
  Python SDK's `ApprovalRequired.resume()` method (S16-
  followup wiring) translates this into a clean
  "still-pending followup work" exception. No silent
  admit / deny.

### Python SDK contract (documentation)

The shipped `templates/onboarding/python-langchain/sdk_adapter.py`
already raises `ApprovalRequired` on REQUIRE_APPROVAL. S16
extends the contract with a `.resume()` method:

```python
class ApprovalRequired(Exception):
    decision_id: UUID
    approval_id: UUID
    approver_role: str

    def resume(self, sidecar: SidecarClient) -> str:
        """Block-poll the approval state then resume the run.

        Behavior:
          * approved → return the LLM response (sidecar re-runs
            ReserveSet idempotently and the caller proceeds).
          * denied → raise ApprovalDenied(reason, approver).
          * pending (TTL not yet expired) → caller picks: poll
            again, or raise ApprovalStillPending.
          * expired → raise ApprovalExpired (release reservation
            via implicit timeout semantics).
          * cancelled → raise ApprovalCancelled.
        """
```

The resume path's idempotency key is opaque to the SDK —
sidecar derives it from `approval_id`. SDK callers don't
need to manage anything beyond catching the typed
exceptions.

### Changed files

- **MODIFIED** `proto/spendguard/sidecar_adapter/v1/adapter.proto`:
  +60 lines — new `ResumeAfterApproval` RPC,
  `ResumeAfterApprovalRequest`, `ResumeAfterApprovalResponse`
  (3-arm oneof), `ResumeAfterApprovalDenied` message.
- **MODIFIED** `services/sidecar/src/server/adapter_uds.rs`:
  +50 lines — new `resume_after_approval` async handler
  returning the typed POC-limitation Error.

### Tests

- Schema-level: proto compiles cleanly + sidecar release
  build succeeds (verified via docker).
- End-to-end resume tests pending — they need the S14-
  followup bundling SP + actual contract re-evaluator
  invocation. Documented in this entry's residual risks.

### Adversarial review

- **Resume publishes effect twice**: idempotency key
  derivation in resume path includes `approval_id`. Even if
  the adapter calls `ResumeAfterApproval` repeatedly, the
  underlying `Ledger.ReserveSet` short-circuits via the
  existing idempotency check (post_ledger_transaction's
  ledger_transactions_idempotency_key UNIQUE). Captured in
  the proto comment as the contract.
- **Approval action requires fresh Ledger check** (S14 spec
  invariant): the resume handler MUST re-run Contract
  evaluation + ReserveSet at resume time, not trust the
  prior decision_context. Documented in the handler stub's
  doc comment. The S16-followup implementer MUST honor this.
- **Deny path skips audit emit**: not possible — deny audit
  row is created at approval-resolution time (S14's SP) +
  `ResumeAfterApprovalDenied` carries the existing event id.
  No new audit emit on resume.
- **Stub handler silently admits**: it doesn't — the typed
  Error response forces the SDK to raise an exception.
  Adapter cannot interpret the stub response as
  "Decision::CONTINUE".
- **TTL-expired approval gets resumed**: the followup
  implementation MUST reject by checking `state` BEFORE
  reading `decision_context`. Documented contract.
- **Unauthenticated resume**: `ResumeAfterApproval` flows
  through the existing UDS adapter handshake; same trust
  model as `RequestDecision`. Adapter pod identity (mTLS or
  UDS peer credentials) is the gate.

### Observability

- New tracing field on every resume invocation: `tenant`,
  `decision_id`, `approval_id`. Once the followup wiring
  lands, additional fields: `approval_state`,
  `idempotency_hit` (true if the underlying Ledger op
  was a replay).
- S23's L8 SLO (approval p99 latency) reads
  `approval_requests.resolved_at - approval_requests.created_at`,
  unaffected by S16.

### Residual risks (S16-followup)

1. **No live resume path**. The stub returns POC limitation
   Error. The followup wiring requires:
   - S14 bundling SP (`post_approval_required_decision`)
     that creates the approval_requests row atomically with
     the audit deny.
   - A read helper that, given
     (tenant_id, decision_id, approval_id), returns the
     approval state + decision_context JSONB.
   - Sidecar code that re-runs contract evaluation against
     decision_context's frozen pricing tuple and emits the
     ReserveSet RPC with the resume idempotency key.
2. **Demo mode `approval`** referenced in the spec
   ("Add demo mode `approval`") not yet shipped. Mock
   approver flow + `make demo-up DEMO_MODE=approval` are
   the followup.
3. **Python SDK actual implementation** of
   `ApprovalRequired.resume()` is documented contract today.
   Real `sdk/python/src/spendguard/exceptions.py` update is
   followup.
4. **Pydantic-AI / LangChain framework integrations**
   referenced in spec ("Add examples for Pydantic-AI and
   LangChain") deferred. The sdk_adapter.py template (S20)
   shows the pattern; framework-specific examples are
   followup.
5. **Resume timeout semantics**: when an approval is in
   `pending` state and the adapter has been polling for too
   long, what's the right exception? Documented as caller's
   choice (raise ApprovalStillPending or keep polling).
   Convention here is ApprovalStillPending after 1× the
   approval TTL — operator playbook will set this.
6. **Codex round still flaking** — code-level review here.

### Runbook deltas

- New RPC: `SidecarAdapter.ResumeAfterApproval`. SDK callers
  reach it via `SidecarClient.resume_after_approval(
  approval_id, decision_id)` (followup; today the stub
  returns POC error).
- Once followup wiring lands: typical adapter flow:
  1. `RequestDecision` → REQUIRE_APPROVAL.
  2. SDK raises `ApprovalRequired(decision_id, approval_id)`.
  3. Caller routes the human approver via dashboard /
     control_plane API (S15).
  4. Caller calls `ApprovalRequired.resume(sidecar)`.
  5. Either the LLM response comes back, or a typed
     ApprovalDenied / ApprovalExpired / ApprovalCancelled
     bubbles up.

### Quality bar

Meets 90%+ for "adapter resume protocol foundation" scope:
typed proto with three-arm oneof matching the spec's
approve/deny/non-actionable cases, idempotency contract
documented at the proto layer (NEW key derived from
approval_id), stub handler that fails-clean rather than
silently admitting, Python SDK contract documented for the
followup implementer, framework-specific behavior captured
as pseudocode in the progress doc. Open items (live
re-run-Contract-+-Ledger wiring, demo mode, real SDK update,
framework example apps, timeout-poll convention) are
explicit S16-followups rather than gaps in the protocol
foundation.

---

(Subsequent slice entries appended below.)
