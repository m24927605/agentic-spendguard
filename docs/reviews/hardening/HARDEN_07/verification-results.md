# HARDEN 07 Verification Results

Date: 2026-05-31
Branch: `harden/HARDEN_07_cargo_helm_migration_verification`

## Cargo

Command:

```bash
scripts/verify-cargo-workspace.sh
CARGO_TARGET_DIR=/tmp/spendguard-harden07-clean-target scripts/verify-cargo-workspace.sh
```

Result: PASS.

- Checked metadata for 30 Cargo manifests.
- Used `--locked` for manifests with committed lockfiles and `--no-deps` for library/service manifests without a committed lockfile.
- Re-runs from a clean detached git worktree so ignored local per-service `Cargo.lock` files cannot mask checkout behavior.
- Built the predictor-upgrade Rust set:
  - `benchmarks/predictor-upgrade/Cargo.toml`
  - `services/canonical_ingest/Cargo.toml`
  - `services/control_plane/Cargo.toml`
  - `services/egress_proxy/Cargo.toml`
  - `services/ledger/Cargo.toml`
  - `services/output_predictor/Cargo.toml`
  - `services/run_cost_projector/Cargo.toml`
  - `services/sidecar/Cargo.toml`
  - `services/stats_aggregator/Cargo.toml`
  - `services/tokenizer/Cargo.toml`
- No Cargo.lock drift remained after verification.
- Runs affected regression tests for canonical_ingest, control_plane, egress_proxy, output_predictor, run_cost_projector, stats_aggregator, and tokenizer.
- Final rerun from commit `dd9d477` passed after fixing the `run_cost_projector` cold-miss race.

## Helm

Command:

```bash
scripts/verify-helm-profiles.sh
```

Result: PASS.

Rendered matrix:

- `demo`
- `demo-networkpolicy`
- `production`
- `production-networkpolicy`
- `production-kms`

Additional checks:

- Rendered manifests contain no plaintext `postgres://` URLs.
- Production KMS control-plane render contains no local signing Secret mount or `control-plane.pem` reference.
- YAML assertions verify every rendered Deployment, Job, and CronJob container has `readOnlyRootFilesystem: true`, `allowPrivilegeEscalation: false`, and capability drop `ALL`.
- YAML assertions verify every rendered database environment variable is sourced from a `secretKeyRef`.

## Migrations

Command:

```bash
scripts/verify-migrations-postgres16.sh
```

Result: PASS.

- Started a fresh `postgres:16-alpine` container.
- Applied all ledger migrations in sorted order: 51 SQL files.
- Applied all canonical_ingest migrations in sorted order: 20 SQL files.
- Applied all control_plane migrations in sorted order: 5 SQL files.
- Smoke checks proved:
  - `audit_outbox` and `tokenizer_t1_samples` exist with prediction columns.
  - `canonical_events` and `canonical_event_replay_dedup` exist with mirror columns.
  - `predictor_plugin_endpoints`, `control_plane_audit_outbox`, and `control_plane_audit_outbox_forwarder_update` policy exist.
- Smoke checks are hard `DO $$ ... RAISE EXCEPTION` assertions, not informational `SELECT` output.

## NetworkPolicy

Command:

```bash
tests/k8s/networkpolicy_egress_chaos.sh
```

Result: PASS.

- Created kind cluster `spendguard-netpol` with default CNI disabled.
- Installed Calico `v3.28.2` so NetworkPolicy enforcement is real.
- Applied the chart's `templates/networkpolicy.yaml` with `networkPolicy.enabled=true`.
- Verified an unlabeled control pod can reach `https://1.1.1.1`, proving the cluster has baseline external egress before attributing denial to NetworkPolicy.
- Verified an enforced app pod can reach the in-cluster egress proxy on port 9000.
- Verified the same enforced app pod cannot reach `https://1.1.1.1` directly.

## Adversarial Review Round 1

Reviewer: separate codex CLI reviewer via AIT-compatible subagent after local `ait run` rejected the documented `--review-mode`/`--base` flags.

Findings fixed in-slice:

- Blocker: migration smoke checks were informational and column checks accepted partial state. Fixed with hard Postgres assertions for all required tables, columns, and RLS policies.
- Major: Cargo verification could depend on ignored local lockfiles. Fixed by re-running the verifier in a clean detached git worktree from `HEAD`.
- Major: Cargo verifier omitted affected tests. Fixed by adding seven focused regression test commands across the predictor upgrade services.
- Major: NetworkPolicy chaos could false-pass if the cluster had no external egress. Fixed by adding an unlabeled control pod external egress check before the enforced deny check.
- Minor: Helm checks were global string greps. Fixed with YAML-level per-workload container security and database secret assertions.

Additional defect found by the clean verifier:

- `run_cost_projector` had a cold-miss race where concurrent first `Project` calls for the same run could each insert a separate `RunState` and bypass the intended per-run mutex. Fixed with `RunStateCache::get_or_insert` and regression coverage; `cargo test --manifest-path services/run_cost_projector/Cargo.toml -- --nocapture` passed with 55 library tests, 5 binary tests, and 3 integration tests.

## Adversarial Review Round 2

Reviewer: separate codex CLI reviewer via AIT-compatible subagent after local `ait run` rejected the documented `--review-mode`/`--base` flags.

Finding fixed in-slice:

- Minor: `scripts/verify-cargo-workspace.sh` used `exec` after registering a cleanup trap, so successful verifier runs leaked detached temp worktrees. Fixed by invoking the child verifier normally and allowing the parent shell's `EXIT` trap to remove the worktree. Final `CARGO_TARGET_DIR=/tmp/spendguard-harden07-clean-target scripts/verify-cargo-workspace.sh` passed, and `git worktree list | rg 'spendguard-cargo-clean'` returned no entries.

## Demo Regression

Command:

```bash
make demo-down
make demo-up DEMO_MODE=default
```

Result: PASS.

- Demo handshake succeeded.
- `release_reservation`, `RequestDecision`, `ConfirmPublishOutcome`, `emit_llm_call_post`, and webhook provider report all completed.
- Phase 2B Step 8 SQL assertions passed with provider-reported commit state.
- Outbox forwarder drained 7/7 audit rows.
- `canonical_events` verification passed with count 5.
- Final rerun after the `run_cost_projector` race fix rebuilt the run-cost-projector image and passed the same assertions.
