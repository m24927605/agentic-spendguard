# GA_08 Command Results

Date: 2026-06-01

| Gate | Result | Evidence |
|---|---|---|
| `benchmarks/ga-load/run.sh --scenario benchmarks/ga-load/scenarios/local-100-tenants.yaml` | PASS | ops 100/100; logical tenants 100; providers 4; canonical decision/outcome 100/100; ledger decision/outcome 100/100; pending 0; failures 0 |
| latency `tokenizer` | PASS | count 100, p50 0.663ms, p95 2.646ms, p99 18.244ms, max 20.318ms |
| latency `output_predictor` | PASS | count 100, p50 0.526ms, p95 3.792ms, p99 26.529ms, max 26.774ms |
| latency `run_cost_projector` | PASS | count 100, p50 1.574ms, p95 27.114ms, p99 48.104ms, max 48.621ms |
| latency `sidecar_decision` | PASS | count 100, p50 22.489ms, p95 71.053ms, p99 142.43ms, max 178.316ms |
| latency `sidecar_confirm_publish_outcome` | PASS | count 100, p50 0.51ms, p95 1.798ms, p99 2.98ms, max 3.043ms |
| latency `sidecar_emit_trace_events` | PASS | count 100, p50 19.98ms, p95 65.63ms, p99 86.305ms, max 128.011ms |
| latency `end_to_end` | PASS | count 100, p50 46.159ms, p95 161.447ms, p99 310.217ms, max 327.412ms |
| `python3 tests/e2e/verify_audit_columns.py --tenant 00000000-0000-4000-8000-000000000001` | PASS | `verify-audit-columns.txt` |
| `psql -d spendguard_canonical -f scripts/db/explain-ga-plans.sql` | PASS | `explain-ga-plans.txt` |
