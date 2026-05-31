# HARDEN_02 Command Results

Date: 2026-05-31 Asia/Taipei
Branch: `harden/HARDEN_02_e2e_real_cluster_validation`

## Build and Unit Gates

| Gate | Result |
|---|---|
| `make -C sdk/python proto` | PASS |
| `make -C sdk/python test` | PASS (`849 passed, 4 skipped`) |
| `cargo build && cargo test` for `services/sidecar` | PASS |
| `cargo build && cargo test` for `services/canonical_ingest` | PASS |
| `cargo build && cargo test` for `services/egress_proxy` | PASS after rerunning one transient p99 timing flake |
| `cargo build && cargo test` for `services/run_cost_projector` | PASS |
| `cargo test --manifest-path services/output_predictor/Cargo.toml breaker_open_skips_predict_without_recording_extra_failure -- --nocapture` | PASS |
| `cargo test -p spendguard-predictor-upgrade-benchmarks` | PASS |

## Template and Cluster Gates

| Gate | Result |
|---|---|
| `helm template charts/spendguard --set chart.profile=demo` | PASS |
| `helm template charts/spendguard --set chart.profile=production -f docs/reviews/hardening/HARDEN_02/kind-production-values.example.yaml` | PASS |
| `kind create cluster --name spendguard-harden02` + `helm install spendguard charts/spendguard -n spendguard --create-namespace --set chart.profile=production -f docs/reviews/hardening/HARDEN_02/kind-production-values.example.yaml --wait=false --timeout 60s` | PASS (`STATUS: deployed`) |

## E2E and Demo Gates

| Gate | Result |
|---|---|
| `E2E_HEALTH_TIMEOUT_S=600 bash tests/e2e/predictor_upgrade.sh` | PASS: 7/7 healthchecked services healthy; canonical mirror columns `11/11` present |
| `make demo-up DEMO_MODE=default` | PASS |
| `python3 tests/e2e/verify_audit_columns.py --tenant 00000000-0000-4000-8000-000000000001` | PASS: `21/21` columns existing, `21/21` populated, verify-chain GREEN |
| `make demo-up DEMO_MODE=m1_benchmark_runaway_loop` | PASS: `RUN_BUDGET_PROJECTION_EXCEEDED` found in canonical events |
| `make demo-up DEMO_MODE=multi_provider_usd` | PASS: all five provider routes verified by egress proxy test |
| `make demo-up DEMO_MODE=agent_real_anthropic` | PASS: mock path used unless `SPENDGUARD_DEMO_REAL_ANTHROPIC=1` and valid key are supplied |
| `make demo-up DEMO_MODE=plugin_c_synthetic` | PASS |

## Benchmark Gate

Command:

```bash
SHIM_DISABLE_LEDGER_LOG=1 docker compose -f benchmarks/runaway-loop/compose.yml up -d --build spendguard-shim
SPENDGUARD_BENCH_SHIM_URL=http://localhost:8090 cargo run --release -p spendguard-predictor-upgrade-benchmarks -- --targets spendguard --output docs/reviews/hardening/HARDEN_02/predictor-benchmark
docker compose -f benchmarks/runaway-loop/compose.yml down -v
```

Result: PASS.

| Burst | Samples | Errors | p50 us | p95 us | p99 us | p99.9 us |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 1000 | 0 | 391 | 447 | 504 | 648 |
| 10 | 1000 | 0 | 1679 | 3083 | 3645 | 3933 |
| 100 | 1000 | 0 | 13247 | 14719 | 15407 | 15599 |

Artifacts:

- `docs/reviews/hardening/HARDEN_02/predictor-benchmark/RESULTS.md`
- `docs/reviews/hardening/HARDEN_02/predictor-benchmark/results.json`

## Issues Found and Fixed During Validation

- Docker Compose race: Postgres became temporarily healthy during initdb; `output-predictor` and `run-cost-projector` could start before final Postgres restart and fail with pool timeouts. Fixed by gating both services on `canonical-seed-init`.
- Demo default claim estimate used an invalid tokenizer version UUID. Fixed to use the seeded tokenizer version ID.
- `make demo-up DEMO_MODE=m1_benchmark_runaway_loop` originally recreated sidecar without the projector URL. Fixed by running the demo container with `--no-deps` after stack bring-up.
- `verify_audit_columns.py` could not run `verify-chain` from the canonical-ingest image. Fixed the demo Dockerfile to copy the binary.
- Benchmark harness originally timed reserve+commit for the decision SLO and included shim logging/threadpool overhead. Fixed to report decision-only reserve/deny latency, keep runaway-loop as the reserve+commit receipt benchmark, disable shim access/audit logging for latency runs, and warm at least two full burst waves.

## AIT Round 1 Fix Evidence

Reviewer: `review_01KSXMST512E6H23FEST1VGJE3`.

| Finding | Fix | Verification |
|---|---|---|
| Production proxy path double-advances projector state | Removed the production default for `egressProxy.runCostProjectorEndpoint`; sidecar remains the only mutating Project caller when explicitly configured, and Helm rejects explicit proxy projector wiring when sidecar projector is configured. | `helm template ... production` renders no proxy projector endpoint; negative Helm gate rejects explicit proxy run-cost endpoint when sidecar projector is set. |
| Run-budget projection is caller-controlled | Added `sidecar.allowUntrustedBudgetMetadata` / `SPENDGUARD_SIDECAR_ALLOW_UNTRUSTED_BUDGET_METADATA`, default false and rejected in production; m1 demo is the only path that enables it. | `cargo test --manifest-path services/sidecar/Cargo.toml`; production negative Helm gate rejects unsafe flag; `make demo-up DEMO_MODE=m1_benchmark_runaway_loop` still passes. |
| Denied projection rows drop aggregator mirrors | Shared ClaimEstimate payload mirror insertion across allow and denied decision payloads. | `cargo test --manifest-path services/sidecar/Cargo.toml claim_estimate_payload_mirrors -- --nocapture`. |

## AIT Round 2 Fix Evidence

Reviewer: `review_01KSXNXYHSRM7JBWP93B9W6TE0`.

| Finding | Fix | Verification |
|---|---|---|
| Projector advances before idempotency is resolved | Sidecar now validates adapter idempotency before Project and derives ProjectRequest.decision_id from a stable SHA-256 of the adapter key. run_cost_projector caches ProjectResponse by decision_id per run and returns replay responses without calling `record_step`. | `cargo test --manifest-path services/run_cost_projector/Cargo.toml` includes `project_is_idempotent_by_decision_id`; `cargo test --manifest-path services/sidecar/Cargo.toml` includes `projector_decision_id_is_stable_bounded_hash`. |
| ClaimEstimate drops authoritative projector audit fields | ALLOW and DENY CloudEvents still take tokenizer/output predictor fields from ClaimEstimate, but projector_response now overrides the 3 run-level audit fields whenever present. | `cargo test --manifest-path services/sidecar/Cargo.toml` includes `projector_response_overrides_claim_estimate_run_fields`. |
| Production budget projection remains disabled but chart advertised it | Production Helm no longer auto-wires sidecar to run_cost_projector. Operators must explicitly set `sidecar.runCostProjectorUrl`; unsafe caller-supplied budget metadata remains rejected in production. | `helm template charts/spendguard --set chart.profile=production -f docs/reviews/hardening/HARDEN_02/kind-production-values.example.yaml` renders without `SPENDGUARD_SIDECAR_RUN_COST_PROJECTOR_URL`; explicit sidecar URL renders when set; unsafe budget metadata negative gate fails. |
| UDS projector mode renders dead sidecar wiring | Removing production auto-wiring means UDS-only run_cost_projector no longer produces a dead TCP sidecar endpoint by default. | Same production Helm render as above; default production output has no sidecar run_cost_projector env var. |

Round 2 command results:

- `cargo build --manifest-path services/run_cost_projector/Cargo.toml`: PASS.
- `cargo build --manifest-path services/sidecar/Cargo.toml`: PASS (existing `schema_bundle_canonical_version` dead-code warning).
- `cargo test --manifest-path services/run_cost_projector/Cargo.toml`: PASS (`52 + 5 + 3` tests).
- `cargo test --manifest-path services/sidecar/Cargo.toml`: PASS (`112 + 6` tests; existing warning).
- `helm template charts/spendguard --set chart.profile=demo`: PASS.
- `helm template charts/spendguard --set chart.profile=production -f docs/reviews/hardening/HARDEN_02/kind-production-values.example.yaml`: PASS; no default sidecar projector URL rendered.
- Negative Helm gate with both `sidecar.runCostProjectorUrl` and `egressProxy.runCostProjectorEndpoint`: FAILS as expected.
- Negative Helm gate with `sidecar.allowUntrustedBudgetMetadata=true` in production: FAILS as expected.
- `make -C deploy/demo demo-up DEMO_MODE=m1_benchmark_runaway_loop`: PASS; `RUN_BUDGET_PROJECTION_EXCEEDED` observed and canonical_events matching count = 1.
- `make -C deploy/demo demo-down`: PASS.

## AIT Round 3 Fix Evidence

Reviewer: `review_01KSXQ6WGVBHEPD276771DW4GN`.

| Finding | Fix | Verification |
|---|---|---|
| Production budget projection cannot fire | When `sidecar.allowUntrustedBudgetMetadata=false`, sidecar now derives `ProjectRequest.budget_remaining_atomic` from authoritative `Ledger.QueryBudgetState` over debit claims and uses the minimum available budget. The demo metadata path remains behind the demo/test gate. | `cargo test --manifest-path services/sidecar/Cargo.toml` includes budget snapshot parsing/clamping tests; m1 demo still passes through the explicit demo gate. |
| Concurrent Project calls undercount run state | run_cost_projector now rechecks the per-run state under lock before committing a Project response. If another distinct decision mutated the run during Signal 1 await, it recomputes from the fresh state instead of returning an undercounted projection. | `cargo test --manifest-path services/run_cost_projector/Cargo.toml` includes `concurrent_distinct_projects_see_serialized_steps`. |
| Caller can rewrite audited prediction policy | Sidecar no longer copies `ClaimEstimate.prediction_policy_used` into signed ALLOW or DENY CloudEvents; the field remains authoritative from `bundle.parsed.prediction_policy`. | `cargo test --manifest-path services/sidecar/Cargo.toml` includes `claim_estimate_cannot_override_contract_prediction_policy`. |

Round 3 command results:

- `cargo build --manifest-path services/run_cost_projector/Cargo.toml`: PASS.
- `cargo build --manifest-path services/sidecar/Cargo.toml`: PASS (existing `schema_bundle_canonical_version` dead-code warning).
- `cargo test --manifest-path services/run_cost_projector/Cargo.toml`: PASS (`53 + 5 + 3` tests).
- `cargo test --manifest-path services/sidecar/Cargo.toml`: PASS (`113 + 6` tests; existing warning).
- `cargo test -p spendguard-predictor-upgrade-benchmarks`: PASS.
- `helm template charts/spendguard --set chart.profile=demo`: PASS.
- `helm template charts/spendguard --set chart.profile=production -f docs/reviews/hardening/HARDEN_02/kind-production-values.example.yaml`: PASS.
- Explicit production sidecar projector URL render: PASS; only `SPENDGUARD_SIDECAR_RUN_COST_PROJECTOR_URL` appears.
- Negative Helm gate with both sidecar and egress proxy projector URLs: FAILS as expected.
- `make -C deploy/demo demo-up DEMO_MODE=m1_benchmark_runaway_loop`: PASS; `RUN_BUDGET_PROJECTION_EXCEEDED` observed and canonical_events matching count = 1.
- `make -C deploy/demo demo-down`: PASS.

## AIT Round 4 Fix Evidence

Reviewer: `review_01KSXRBRK7H72G9A5NXA9PTP7A`.

| Finding | Fix | Verification |
|---|---|---|
| Ledger available balance double-counts prior run spend | run_cost_projector now keeps `projection_atomic = cumulative + this_call + predicted_remaining` for audit forensics, but compares only the future commitment (`this_call + predicted_remaining`) against the live ledger `available_atomic` budget snapshot. The m1 runaway-loop harness now decrements the live budget balance before each call, matching ledger semantics. | `cargo test --manifest-path services/run_cost_projector/Cargo.toml` includes `live_available_budget_does_not_double_count_prior_spend`; `make -C deploy/demo demo-up DEMO_MODE=m1_benchmark_runaway_loop` still emits `RUN_BUDGET_PROJECTION_EXCEEDED` and lands it in `canonical_events`. |
| Projector can change replayed decisions after cache loss | Ledger `QueryDecisionOutcome` now accepts an idempotency key lookup and returns durable replay metadata from the original reserve/deny audit row. Sidecar checks that durable replay before any mutating Project call, reconstructing the original CONTINUE or denied RUN_* decision without advancing projector state. | `cargo test --manifest-path services/ledger/Cargo.toml` includes replay metadata extraction coverage; `cargo test --manifest-path services/sidecar/Cargo.toml` includes `idempotency_replay_decision_kind_preserves_projection_stop`. |

Round 4 command results:

- `git diff --check`: PASS.
- `cargo build --manifest-path services/run_cost_projector/Cargo.toml`: PASS.
- `cargo build --manifest-path services/sidecar/Cargo.toml`: PASS (existing `schema_bundle_canonical_version` dead-code warning).
- `cargo build --manifest-path services/ledger/Cargo.toml`: PASS (existing `estimated` warning).
- `cargo test --manifest-path services/run_cost_projector/Cargo.toml`: PASS (`54 + 5 + 3` tests).
- `cargo test --manifest-path services/sidecar/Cargo.toml`: PASS (`114 + 6` tests; existing warning).
- `cargo test --manifest-path services/ledger/Cargo.toml`: PASS (`14` tests; existing warning).
- `cargo test -p spendguard-predictor-upgrade-benchmarks`: PASS.
- `helm template charts/spendguard --set chart.profile=demo`: PASS.
- `helm template charts/spendguard --set chart.profile=production -f docs/reviews/hardening/HARDEN_02/kind-production-values.example.yaml`: PASS.
- `make -C deploy/demo demo-up DEMO_MODE=m1_benchmark_runaway_loop`: PASS; `RUN_BUDGET_PROJECTION_EXCEEDED` observed and canonical_events matching count = 1.
- `OPENAI_API_KEY=dummy ANTHROPIC_API_KEY=dummy make -C deploy/demo demo-down`: PASS.

## AIT Round 5 and Staff+ Arbitration

Reviewer: `review_01KSXT8P6C2XT2Y2Z4G5PEJ1FD`.

Round 5 returned two remaining high-severity findings:

- Durable idempotency replay accepted the adapter key without validating that the replayed request matched the original request.
- The production kind values example deployed `run_cost_projector` without wiring the sidecar to call it.

Per HARDEN_02 §12, codex review iteration stopped after round 5 and a Staff+ panel arbitrated the remaining findings.

| Panelist | Finding 1 decision | Finding 2 decision |
|---|---|---|
| Software Architect | Fix in-slice with request fingerprint validation before replay | Fix the production example explicitly; avoid broad chart auto-wiring |
| Backend Architect | Fix in-slice across durable replay and in-memory cache | Set `sidecar.runCostProjectorUrl` in the production example |
| Security Engineer | Fix in-slice and fail closed on missing or mismatched fingerprint | Keep production posture explicit and non-implicit |
| Database Optimizer | Fix in-slice using stored audit metadata as replay ledger input | Avoid chart-wide behavior changes |
| SpendGuard Predictor Domain Expert | Fix in-slice; duplicate adapter keys must not replay different claims | Wire the example so projection can actually fire |

Final arbitration: fix both findings in HARDEN_02, with no AIT round 6. The panel decision is final for merge readiness.

Fixes applied:

- Sidecar computes a stable idempotency request fingerprint from tenant, region, and the encoded `DecisionRequest`.
- Sidecar writes the fingerprint into reserve and denied decision CloudEvents as `idempotency_request_fingerprint`.
- Ledger `QueryDecisionOutcome` returns the stored fingerprint with durable replay metadata.
- Sidecar rejects durable replay before any projector mutation when the stored fingerprint is missing or mismatched.
- Sidecar in-memory idempotency cache now stores the same fingerprint and rejects same-key/different-request conflicts.
- `kind-production-values.example.yaml` now explicitly sets `sidecar.runCostProjectorUrl: https://spendguard-spendguard-run-cost-projector:50055`.

Staff+ arbitration verification:

- `make -C sdk/python proto`: PASS; no tracked Python generated diff.
- `cargo test --manifest-path services/ledger/Cargo.toml`: PASS (`14` tests; existing `estimated` warning).
- `cargo test --manifest-path services/sidecar/Cargo.toml`: PASS (`116 + 6` tests; existing `schema_bundle_canonical_version` warning).
- `cargo test --manifest-path services/run_cost_projector/Cargo.toml`: PASS (`54 + 5 + 3` tests).
- `cargo test -p spendguard-predictor-upgrade-benchmarks`: PASS.
- `helm template charts/spendguard --set chart.profile=demo`: PASS.
- `helm template charts/spendguard --set chart.profile=production -f docs/reviews/hardening/HARDEN_02/kind-production-values.example.yaml`: PASS; renders `SPENDGUARD_SIDECAR_RUN_COST_PROJECTOR_URL=https://spendguard-spendguard-run-cost-projector:50055`.
- `make -C deploy/demo demo-up DEMO_MODE=m1_benchmark_runaway_loop`: PASS; `RUN_BUDGET_PROJECTION_EXCEEDED` observed and canonical_events matching count = 1.
- `OPENAI_API_KEY=dummy ANTHROPIC_API_KEY=dummy make -C deploy/demo demo-down`: PASS.
- `git diff --check`: PASS.
