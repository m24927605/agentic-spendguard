# GA_08 Command Results

Date: 2026-06-01

Scope: local real-stack smoke and DB-plan gate. Contract §14 latency certification remains the `spendguard-predictor-upgrade-benchmarks` gate.

| Gate | Result | Evidence |
|---|---|---|
| `benchmarks/ga-load/run.sh --scenario benchmarks/ga-load/scenarios/local-100-tenants.yaml` | PASS | ops 100/100; logical tenants 100; providers 4; canonical decision/outcome 100/100; ledger decision/outcome 100/100; pending 0; failures 0 |
| local smoke latency `tokenizer` | PASS | count 100, p50 0.544ms, p95 2.402ms, p99 10.071ms, max 10.158ms |
| local smoke latency `output_predictor` | PASS | count 100, p50 0.431ms, p95 3.519ms, p99 14.435ms, max 15.11ms |
| local smoke latency `run_cost_projector` | PASS | count 100, p50 1.168ms, p95 6.762ms, p99 51.073ms, max 53.157ms |
| local smoke latency `sidecar_decision` | PASS | count 100, p50 19.5ms, p95 49.629ms, p99 101.917ms, max 121.008ms |
| local smoke latency `sidecar_confirm_publish_outcome` | PASS | count 100, p50 0.434ms, p95 0.865ms, p99 1.048ms, max 1.108ms |
| local smoke latency `sidecar_emit_trace_events` | PASS | count 100, p50 18.483ms, p95 42.373ms, p99 83.512ms, max 94.344ms |
| local smoke latency `end_to_end` | PASS | count 100, p50 40.455ms, p95 87.934ms, p99 256.852ms, max 277.087ms |
| `python3 tests/e2e/verify_audit_columns.py --tenant 00000000-0000-4000-8000-000000000001` | PASS | `verify-audit-columns.txt` |
| `psql -d spendguard_canonical -f scripts/db/explain-ga-plans.sql` | PASS | `explain-ga-plans.txt` |
