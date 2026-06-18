# HARDEN_02 E2E Runbook

This runbook is the reproducible gate sequence for HARDEN_02. It intentionally uses real Docker Compose, Helm, kind, and release benchmark commands.

## Compose E2E

```bash
E2E_HEALTH_TIMEOUT_S=600 bash tests/e2e/predictor_upgrade.sh
python3 tests/e2e/verify_audit_columns.py --tenant 00000000-0000-4000-8000-000000000001
bash tests/e2e/predictor_upgrade.sh --down
```

Expected:

- `spendguard-postgres`, `endpoint-catalog`, `sidecar`, `tokenizer`, `output-predictor`, `run-cost-projector`, and `stats-aggregator` all reach `healthy`.
- `canonical_events` has the predictor mirror columns.
- `verify_audit_columns.py` reports all predictor/audit columns present and populated after a demo run.

## Demo Modes

```bash
make -C deploy/demo demo-up DEMO_MODE=default
python3 tests/e2e/verify_audit_columns.py --tenant 00000000-0000-4000-8000-000000000001
make -C deploy/demo demo-down

make -C deploy/demo demo-up DEMO_MODE=m1_benchmark_runaway_loop
make -C deploy/demo demo-down

make -C deploy/demo demo-up DEMO_MODE=multi_provider_usd
make -C deploy/demo demo-down

make -C deploy/demo demo-up DEMO_MODE=agent_real_anthropic
make -C deploy/demo demo-down

make -C deploy/demo demo-up DEMO_MODE=plugin_c_synthetic
```

Expected:

- `default`: Step 8 verifier and outbox closure pass.
- `m1_benchmark_runaway_loop`: canonical event payload contains `RUN_BUDGET_PROJECTION_EXCEEDED`.
- `multi_provider_usd`: deterministic routing table covers OpenAI, Anthropic, Bedrock, Vertex, and Azure OpenAI. This offline route intentionally skips outbox closure when no provider keys are supplied.
- `agent_real_anthropic`: uses the mock path unless `SPENDGUARD_DEMO_REAL_ANTHROPIC=1` and a valid `ANTHROPIC_API_KEY` are supplied.
- `plugin_c_synthetic`: Strategy C breaker-open regression test passes.

## Production Helm on kind

```bash
kind create cluster --name spendguard-harden02
helm install spendguard charts/spendguard \
  -n spendguard \
  --create-namespace \
  --set chart.profile=production \
  -f docs/internal/reviews/hardening/HARDEN_02/kind-production-values.example.yaml \
  --wait=false \
  --timeout 60s
helm status spendguard -n spendguard
helm uninstall spendguard -n spendguard
kind delete cluster --name spendguard-harden02
```

Expected: `helm status` reports `STATUS: deployed`.

## Predictor Benchmark

```bash
SHIM_DISABLE_LEDGER_LOG=1 docker compose -f benchmarks/runaway-loop/compose.yml up -d --build spendguard-shim
SPENDGUARD_BENCH_SHIM_URL=http://localhost:8090 \
  cargo run --release -p spendguard-predictor-upgrade-benchmarks -- \
    --targets spendguard \
    --output docs/internal/reviews/hardening/HARDEN_02/predictor-benchmark
docker compose -f benchmarks/runaway-loop/compose.yml down -v
```

Expected: command exits 0 and writes `RESULTS.md` plus `results.json`; SpendGuard p99 is below `50_000us` for bursts `1,10,100`.
