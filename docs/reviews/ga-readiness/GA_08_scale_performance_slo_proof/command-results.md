# GA_08 Command Results

Date: 2026-06-01

| Gate | Result | Evidence |
|---|---|---|
| `benchmarks/ga-load/run.sh --scenario benchmarks/ga-load/scenarios/local-100-tenants.yaml` | PASS | ops 100/100; logical tenants 100; providers 4; canonical delta 200; pending 0; failures 0 |
| latency `tokenizer` | PASS | count 100, p50 0.476ms, p95 1.429ms, p99 11.205ms, max 12.301ms |
| latency `output_predictor` | PASS | count 100, p50 0.404ms, p95 4.153ms, p99 15.621ms, max 15.728ms |
| latency `run_cost_projector` | PASS | count 100, p50 1.113ms, p95 13.486ms, p99 39.343ms, max 41.088ms |
| latency `sidecar_decision` | PASS | count 100, p50 16.723ms, p95 55.547ms, p99 95.676ms, max 113.493ms |
| latency `sidecar_confirm_publish_outcome` | PASS | count 100, p50 0.334ms, p95 0.938ms, p99 1.215ms, max 2.349ms |
| latency `sidecar_emit_trace_events` | PASS | count 100, p50 16.03ms, p95 40.35ms, p99 54.237ms, max 72.43ms |
| latency `end_to_end` | PASS | count 100, p50 34.342ms, p95 115.698ms, p99 208.951ms, max 220.552ms |
| `python3 tests/e2e/verify_audit_columns.py --tenant 00000000-0000-4000-8000-000000000001` | PASS | `verify-audit-columns.txt` |
| `psql -d spendguard_canonical -f scripts/db/explain-ga-plans.sql` | PASS | `explain-ga-plans.txt` |
