# GA_08 Command Results

Date: 2026-06-01

Scope: local real-stack smoke and DB-plan gate. Contract §14 latency certification remains the `spendguard-predictor-upgrade-benchmarks` gate.

| Gate | Result | Evidence |
|---|---|---|
| `benchmarks/ga-load/run.sh --scenario benchmarks/ga-load/scenarios/local-100-tenants.yaml` | PASS | ops 100/100; logical tenants 100; providers 4; canonical decision/outcome 100/100; ledger decision/outcome 100/100; pending 0; failures 0 |
| local smoke latency `tokenizer` | PASS | count 100, p50 0.858ms, p95 10.842ms, p99 16.059ms, max 17.567ms |
| local smoke latency `output_predictor` | PASS | count 100, p50 0.836ms, p95 10.754ms, p99 45.87ms, max 46.855ms |
| local smoke latency `run_cost_projector` | PASS | count 100, p50 2.423ms, p95 65.452ms, p99 82.577ms, max 143.997ms |
| local smoke latency `sidecar_decision` | PASS | count 100, p50 34.278ms, p95 262.625ms, p99 288.978ms, max 490.564ms |
| local smoke latency `sidecar_confirm_publish_outcome` | PASS | count 100, p50 0.85ms, p95 3.774ms, p99 7.581ms, max 10.397ms |
| local smoke latency `sidecar_emit_trace_events` | PASS | count 100, p50 30.178ms, p95 167.317ms, p99 362.628ms, max 377.267ms |
| local smoke latency `end_to_end` | PASS | count 100, p50 72.619ms, p95 458.178ms, p99 779.052ms, max 915.346ms |
| `python3 tests/e2e/verify_audit_columns.py --tenant 00000000-0000-4000-8000-000000000001` | PASS | `verify-audit-columns.txt` |
| `psql -d spendguard_canonical -f scripts/db/explain-ga-plans.sql` | PASS | `explain-ga-plans.txt` |
