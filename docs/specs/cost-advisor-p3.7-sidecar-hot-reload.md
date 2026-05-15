# Cost Advisor P3.7 — Sidecar Contract Bundle Hot-Reload

> **Date**: 2026-05-15
> **Status**: shipped — main HEAD `26da60f`
> **Branch / commit**: `feat/cost-advisor-p3.7-sidecar-hot-reload` → `d522f65` → merge `26da60f`
> **Demo**: `DEMO_MODE=cost_advisor make demo-up` PASS end-to-end on fresh volume; sidecar logs structured `event=hot_reload_swapped` exactly once per rotation
> **Codex iteration**: r1 RED (2 P1 + 4 P2 + 5 P3) → r2 GREEN
> **Closes**: the last gap in Cost Advisor v0.1 closed loop — rule fires → finding → approval → resolve → bundle rotation → **sidecar uses new contract** (no restart)

---

## 1. Context — what gap this closes

Before CA-P3.7 the closed-loop chain ended at the bundle file. CA-P3.5 wired bundle_registry to LISTEN on `approval_requests_state_change` NOTIFY and rotate the on-disk `.tgz` + `.sig` + `runtime.env` within ~1s of operator approval. CA-P3.6 added the dashboard UI for that approval. But the sidecar in v0.1 loaded its `CachedContractBundle` exactly once at startup via `bootstrap::bundles::install_contract_bundle`, so the new bytes were ignored until an operator-driven pod restart.

The pre-P3.7 wedge could only be demonstrated as "bundle bytes land + new hash published"; the actually-running sidecar continued evaluating against the old contract. CA-P3.7 makes the loop *mechanically* close: the sidecar atomically swaps its cached bundle within ~500ms of `runtime.env` being rewritten, so a freshly-approved patch starts gating decisions in <2s end-to-end.

The slice does **NOT** address:

- **Multi-budget patches at non-zero array index** — CA-P3.1's "test op pins index 0" limitation is unchanged. CA-P3.7 doesn't touch the patch validator or the contract DSL surface.
- **Schema bundle hot-reload** — only the contract bundle is watched; `state.inner.schema_bundle` is still install-once. Schema changes are far rarer and the same pattern can extend trivially when needed.
- **Per-rule TTL derivation from contract** — `state.inner.reservation_ttl_seconds` is still env-only (`SPENDGUARD_SIDECAR_RESERVATION_TTL_SECONDS`); the contract's `reservation_ttl_seconds` field is parsed but not consumed at decision time. This is a Contract §7 deferred item, not P3.7 scope.

---

## 2. Design

### 2.1 What to watch

The authoritative pointer to the active bundle is `runtime.env`'s line:

```
SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX=<sha256-hex>
```

`bundle_registry::bundle::update_runtime_env` (`services/bundle_registry/src/bundle.rs:126`) rewrites this line **LAST** in its atomic publish sequence:

```
1. write new .tgz       (atomic tempfile + rename)
2. write new .sig       (atomic tempfile + rename)
3. write new runtime.env (atomic tempfile + rename) ← we watch this
```

By the time the watcher sees a new hash, the matching `.tgz` is already durable on disk. Watching the `.tgz` mtime directly would race the writer mid-publish (TOCTOU between `.tgz` rotation and `.sig` rotation).

### 2.2 Polling vs inotify

We chose **polling at 500ms** over `inotify`/`notify` crate. Reasons:

- The bundles volume is a shared docker volume in the demo and a `ReadWriteOnce` PV in Helm. inotify on shared volumes has known unreliability (especially across Docker-for-Mac's VFS boundary, NFS/CSI underneath).
- A 500ms tick reading a ~200B file is bounded, observable, predictable. Worst-case latency is well below upstream NOTIFY + tar repack (~1s combined in the demo).
- The watcher's hot path is a tokio `read_to_string` of a tiny file, then a string compare. On cache hit (steady state) the tick is ~50µs total. No measurable runtime cost.

### 2.3 Reload semantics

On hash mismatch:

1. **Re-use `bootstrap::bundles::load_contract_bundle`** unchanged. It re-reads the `.tgz` from disk, recomputes sha256, verifies it matches the new expected hash, re-parses `contract.yaml` structurally. Fails closed if any step rejects.
2. **Re-use `install_contract_bundle`** which writes the new `Arc<CachedContractBundle>` into `state.inner.contract_bundle: RwLock<Option<...>>`. The `RwLock` write is fast — the parsed contract is `Arc<Contract>` (`SharedContract`), so the swap is pointer-sized.
3. **Fail-closed on every error path**: partial-write window (sha256 mismatch between runtime.env's new hash and the still-being-written `.tgz`), YAML parse failure, signature file missing — all preserve the previously installed bundle, emit a `warn!` with `event=hot_reload_load_failed`, and retry on the next tick.

### 2.4 In-flight decision pinning

The Contract §pinning invariant ("In-flight requests use pinned old bundle") is preserved by the existing hot path in `services/sidecar/src/decision/transaction.rs:184-189`:

```rust
let bundle = state
    .inner
    .contract_bundle
    .read()
    .clone()
    .ok_or_else(|| DomainError::DecisionStage("no contract bundle loaded".into()))?;
```

`.read().clone()` clones the `CachedContractBundle` struct, whose `parsed: SharedContract` field is `Arc<Contract>`. Once a decision has its clone of the `Arc`, a subsequent `install_contract_bundle` (which writes a new `Some(...)` into the `RwLock`) does **not** affect it — the old `Arc` keeps its refcount alive until the request finishes.

The same pattern holds for `services/sidecar/src/server/adapter_uds.rs:173` (resume-after-approval path).

No change was required to enforce pinning; CA-P3.7 inherits it for free.

---

## 3. Components

| File | Lines added | Purpose |
|---|---|---|
| `services/sidecar/src/bootstrap/hot_reload.rs` | +354 (new) | Watcher module: poll loop, async runtime.env reader, blocking-pool bundle loader, runtime.env parser + 11 unit tests |
| `services/sidecar/src/config.rs` | +24 | `runtime_env_path` (default `/var/lib/spendguard/bundles/runtime.env`), `hot_reload_poll_ms` (default 500; 0 disables) |
| `services/sidecar/src/bootstrap/mod.rs` | +1 | `pub mod hot_reload;` |
| `services/sidecar/src/main.rs` | +48 | `hot_reload::spawn_loop(&cfg, state.clone())?` after `install_bundles`; new `GET /contract` JSON endpoint on existing health server |
| `deploy/demo/compose.yaml` | +6 | `cost-advisor-demo.depends_on.sidecar: service_healthy` |
| `deploy/demo/Makefile` | +7 | `DEMO_MODE=cost_advisor` now brings up `canonical-ingest` + `sidecar` (previously omitted — hot-reload path was untestable via demo) |
| `deploy/demo/cost_advisor_demo.sh` | +88 | New step 6: capture pre-rotation sidecar baseline hash via `/contract`; after step-5 file rotation poll `/contract` for ≤5s; cross-check vs runtime.env on disk |

---

## 4. Watcher lifecycle

```
main()
  ├─ Config::from_env()
  ├─ install_bundles()              ← parses CONTRACT_BUNDLE_ID + verifies hash + installs
  ├─ hot_reload::spawn_loop()       ← P3.7
  │   ├─ uuid::Uuid::parse_str(contract_bundle_id)  ← startup-fatal on bad UUID (P1-2 fix)
  │   ├─ tokio::spawn(async move {
  │   │     loop {
  │   │         tokio::time::sleep(500ms).await;
  │   │         if state.is_draining() { return; }       ← best-effort, see §6
  │   │         match tick(...).await {
  │   │             Ok(_) | Err(_) => continue,           ← errors are non-fatal
  │   │         }
  │   │     }
  │   │ })
  │   └─ Ok(())
  ├─ catalog::refresh_once() + spawn refresh loop
  ├─ run_health_server() (now serves /contract too)
  ├─ run_metrics_server()
  └─ tonic::Server::serve(...)
```

`tick()` itself:

```
1. tokio::fs::read_to_string(runtime.env)              ← async, ~50µs warm
2. parse_runtime_env_hash(contents)                    ← pure
3. compare against state.inner.contract_bundle.read().bundle_hash
4. on mismatch:
   tokio::task::spawn_blocking(|| load_contract_bundle(...))  ← P1-1 fix
     - std::fs::read(.tgz)                             ← sync, 10-30ms
     - Sha256 verify
     - read .sig metadata
     - read .metadata.json
     - contract::parse_from_tgz()                      ← tar+gzip+yaml parse
   .await
5. install_contract_bundle()                           ← RwLock write, <10µs
6. log structured `event=hot_reload_swapped`
```

---

## 5. `/contract` observability endpoint

Added to the existing health server (port 8080 by default, or `SPENDGUARD_SIDECAR_HEALTH_ADDR`). Returns:

```json
{"bundle_id":"11111111-1111-4111-8111-111111111111","hash_hex":"c15367f0d6cce25f..."}
```

When no bundle is loaded (boot race window):

```json
{"bundle_id":null,"hash_hex":null}
```

**Security posture**: unauthenticated, matches `/healthz` + `/readyz` on the same port. Surfaces only operational telemetry — `bundle_id` is a UUID (low sensitivity) and `hash_hex` is the sha256 of a signed bundle (reveals deployed version, no PII). Same level as a pod label. In Helm production, NetworkPolicy should scope the health port to kubelet + monitoring namespace; that's the same recommendation that already applies to `/healthz`.

---

## 6. Fail-closed paths

| Failure mode | Behavior | Visibility |
|---|---|---|
| `runtime.env` missing | `tokio::fs::read_to_string` returns `Err` | `debug!` log, tick retries next interval |
| `runtime.env` has no `SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_HASH_HEX` line | `parse_runtime_env_hash` returns `None` | Silent skip (steady state for unconfigured operator) |
| Hash unchanged | String compare matches | Silent return — most common path |
| Hash changed, but `.tgz` not yet on disk | `std::fs::read(.tgz)` returns `Err` | `warn!` `event=hot_reload_load_failed`, previous bundle stays, retry next tick |
| Hash changed, `.tgz` exists but sha256 doesn't match (partial-write window) | `load_contract_bundle` returns `BundleSignatureInvalid` | `warn!` `event=hot_reload_load_failed`, previous bundle stays, retry next tick |
| Hash changed, `.tgz` valid, but `contract.yaml` structurally invalid | `contract::parse_from_tgz` returns `Err` | `warn!` `event=hot_reload_load_failed`, previous bundle stays. Steady-state pathological case — operator must intervene |
| `spawn_blocking` task panics | `JoinError` propagated as `anyhow::Context` | `debug!` log, retry next tick |
| Sidecar draining (SIGTERM received) | `state.is_draining()` true, watcher loop returns | Watcher exits; in-flight tick may still complete the swap (harmless — RPC server is shutting down) |
| Misconfigured `SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_ID` (not a UUID) | `uuid::Uuid::parse_str` returns `Err` at `spawn_loop` | **Startup-fatal** — main() exits with context (P1-2 fix) |

The sidecar never serves decisions against a half-loaded bundle, and never silently disables hot-reload at runtime.

---

## 7. Codex r1 review — findings + fixes

Adversarial review on the staged diff (~470 lines). Stopping rule per project memory `feedback_codex_iteration_pattern.md`: fix P1 + critical-path P2; stop on remaining P2/P3.

### P1 fixes (in-slice)

**P1-1: synchronous file I/O blocking tokio runtime.** The initial implementation called `std::fs::read_to_string(runtime.env)` + `load_contract_bundle` (which does `std::fs::read(.tgz)` + sha256 + tar+yaml parse, 10-30ms sync work) directly inside the `tokio::spawn` async block. On the demo's 2-vCPU box this would stall a tokio worker thread for the duration. **Fix**: `tokio::fs::read_to_string` for the runtime.env hot path (tiny file, async), and `tokio::task::spawn_blocking` wrapping `load_contract_bundle` for the rare mismatch path.

**P1-2: silent watcher death on bad UUID.** The initial implementation parsed `contract_bundle_id` *inside* the spawned task and returned silently on parse error. If an operator misconfigures `SPENDGUARD_SIDECAR_CONTRACT_BUNDLE_ID` in Helm, the sidecar would start, /readyz would report ready, /contract would serve the startup hash — and no subsequent rotation would ever apply. The error would surface only on manual diff of `/contract` vs `runtime.env`. **Fix**: parse the UUID at `spawn_loop` entry and return `Result<()>`; main propagates with `?` so a bad UUID is startup-fatal.

### P2 fixes (in-slice)

**P2-4: redundant demo regression detector.** The initial demo's step 6 asserted `SIDECAR_BASELINE_HASH != NEW_HASH` to "catch the bit-identical bytes no-op case". But that case is filtered upstream by `apply.rs:88-95` ("Idempotent re-run: the patch produced bit-identical bytes... skipping the disk write") — bundle_registry never rewrites `runtime.env` on a no-op, so step 5's "hash didn't change in 10s" already catches it. The step 6 check was dead detection. **Fix**: removed the redundant `if` and documented the upstream filter in a comment.

### P2 / P3 documented inline, NOT fixed (deferred or non-issues)

**P2-1: drain coordination is best-effort, not formally wired.** The watcher's `JoinHandle` is dropped at `tokio::spawn` so `drain::run_drain` never awaits it. An in-flight tick can complete a swap during shutdown. **Decision**: documented as best-effort in the module doc + the `is_draining()` comment. Wiring through a `tokio::sync::watch` channel adds complexity for negligible benefit — the worst case is a successful swap right as the RPC server stops accepting traffic. Future slice can revisit if drain becomes load-bearing.

**P2-2: `parse_runtime_env_hash` quote handling has theoretical edge cases.** Codex confirmed all paths fail closed at `load_contract_bundle`'s hex decode — the string-compare guards any malformed value. **Decision**: no fix needed.

**P2-3: `/contract` unauthenticated on shared health port.** Identical posture to `/healthz` + `/readyz`. Information leak is operational telemetry only (bundle_id + hash, no PII). **Decision**: documented in §5 above; NetworkPolicy recommendation applies to the whole port, not /contract specifically.

**P3-1 to P3-5**: hygiene (sync `load_contract_bundle` still sync inside `spawn_blocking` — by design), allocation cost in `hex::encode` on every tick (~32B × 2 Hz = noise), duplicate-key warning (parser is defensive vs adversarial files), poll jitter (not needed at 500ms cadence), schema bundle hot-reload (out of scope; trivial extension when needed). **Decision**: all 5 are followup-worthy at most; none affect correctness or shipped surface.

---

## 8. Demo verification

`DEMO_MODE=cost_advisor make demo-up` on fresh volume:

```
step 0: pre-check for prior demo state
step 1: seed 10 reservations + capture pre-rotation baselines:
        OLD_HASH (file)       = d6caa7ed...   (matches sidecar's loaded hash)
        SIDECAR_BASELINE_HASH = d6caa7ed...   ← captured BEFORE step 4 (P3.7)
step 2: spendguard-advise --write-proposals → 1 finding, 2-op patch inserted
step 3: verify cost_findings + cost_findings_id_keys + approval_requests rows
step 4: dashboard GET → state=pending; dashboard POST resolve → approved
step 5: poll runtime.env for hash change (was 10s budget; observed in 1s):
        d6caa7ed... → c15367f0...
        + extract contract.yaml from rotated .tgz: reservation_ttl_seconds=45
step 6 (CA-P3.7): poll sidecar:8080/contract until hash converges:
        loop: GET /contract → c15367f0... (matches NEW_HASH on first iteration)
        observed convergence latency: <500ms (1 watcher tick)
        cross-check: runtime.env on disk == sidecar /contract response
PASS — closed loop end-to-end
```

Sidecar logs prove exactly-once swap firing:

```
{"level":"INFO","fields":{"message":"CA-P3.7: hot-reload watcher starting", ...,"poll_ms":500}}
{"level":"INFO","fields":{"message":"CA-P3.7: contract bundle hot-reloaded",
  "event":"hot_reload_swapped",
  "previous_bundle_id":"Some(11111111-1111-4111-8111-111111111111)",
  "new_bundle_id":"11111111-1111-4111-8111-111111111111",
  "new_bundle_hash_hex":"c15367f0d6cce25f...",
  "previous_bundle_hash_hex":"Some(\"d6caa7ed...\")"}}
```

Unit tests: 11 in `hot_reload::tests` covering `parse_runtime_env_hash` edge cases (export prefix, single/double quotes, comments, blank lines, missing key, empty value, substring guards, duplicates) + 2 file-system round-trip tests.

---

## 9. Deferred items (NOT shipped in CA-P3.7)

### Within-slice followups (P2/P3 documented above)

- Formal drain coordination via `tokio::sync::watch` instead of `is_draining()` poll
- `/metrics` counter for `hot_reload_swapped` / `hot_reload_load_failed` (currently log-only)
- Duplicate-key warn log on adversarial runtime.env
- Schema bundle hot-reload (same pattern; trivial when needed)
- Demo extension: actually re-issue a sidecar Decision RPC before/after reload and assert behavioral diff (would require a patch that changes a contract rule outcome, not just the TTL). Current demo verifies `/contract` hash convergence; behavioral diff is implied but not explicit.

### Adjacent product gaps (separate slices)

- **CA-P3.8 (proposed)**: multi-budget index pinning. Today the rule emits patches at `/spec/budgets/0/...`; with non-zero offending budget index, the `test` op rejects and the operator must reject + hand-rebuild the bundle. Requires `patch_validator` + rule SQL changes; out of CA-P3.7 scope.
- **CA-P2 baseline refresher**: keeps the `cost_baselines` window rolling so `idle_reservation_rate_v1` doesn't drift after 28d.
- **CA-P2 Tier-3 narrative wrapper**: post-finding LLM narrative for operator UX. Already-LOCKED design in `cost-advisor-spec.md` §13.
- **Phase 5 deferred Helm work**: `/contract` endpoint should be added to the chart's monitoring NetworkPolicy. Tracked under issue #3 (Helm prod env mapping).

---

## 10. References

- `services/sidecar/src/bootstrap/hot_reload.rs` — implementation + tests
- `services/sidecar/src/main.rs:144-158, 479-525` — wiring + /contract endpoint
- `services/sidecar/src/config.rs:120-135` — config fields
- `services/sidecar/src/decision/transaction.rs:184-189` — in-flight pinning hot path
- `services/bundle_registry/src/apply.rs:88-118` — atomic write order (the contract this slice depends on)
- `services/bundle_registry/src/bundle.rs:126-153` — runtime.env rewriter (the file this slice watches)
- `deploy/demo/cost_advisor_demo.sh` — step 1 baseline capture + step 6 hot-reload assertion
- `docs/specs/cost-advisor-spec.md` — Cost Advisor v0.1 spec (CA-P3.7 referenced under §closed-loop completion in §0)
- `services/cost_advisor/docs/control-plane-integration.md` — integration design (sidecar hot-reload listed as P3.7 in §7)
- Commit `d522f65` + merge `26da60f` on `main` (2026-05-15)
