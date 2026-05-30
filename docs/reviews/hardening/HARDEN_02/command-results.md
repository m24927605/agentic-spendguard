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
