# GA_08 Command Results

Date: 2026-06-01

Scope: local real-stack smoke and DB-plan gate. Contract §14 latency certification remains the `spendguard-predictor-upgrade-benchmarks` gate.

| Gate | Result | Evidence |
|---|---|---|
| `benchmarks/ga-load/run.sh --scenario benchmarks/ga-load/scenarios/local-100-tenants.yaml` | PASS | ops 100/100; logical tenants 100; providers 4; canonical decision/outcome 100/100; ledger decision/outcome 100/100; pending 0; failures 0 |
| local smoke latency `tokenizer` | PASS | count 100, p50 1.045ms, p95 4.531ms, p99 22.109ms, max 23.902ms |
| local smoke latency `output_predictor` | PASS | count 100, p50 0.952ms, p95 6.033ms, p99 24.174ms, max 33.209ms |
| local smoke latency `run_cost_projector` | PASS | count 100, p50 3.122ms, p95 12.517ms, p99 69.283ms, max 74.357ms |
| local smoke latency `sidecar_decision` | PASS | count 100, p50 40.024ms, p95 111.25ms, p99 264.867ms, max 321.524ms |
| local smoke latency `sidecar_confirm_publish_outcome` | PASS | count 100, p50 0.91ms, p95 2.443ms, p99 3.893ms, max 4.751ms |
| local smoke latency `sidecar_emit_trace_events` | PASS | count 100, p50 36.965ms, p95 89.228ms, p99 133.125ms, max 185.751ms |
| local smoke latency `end_to_end` | PASS | count 100, p50 84.776ms, p95 214.78ms, p99 491.592ms, max 510.357ms |
| `python3 tests/e2e/verify_audit_columns.py --tenant 00000000-0000-4000-8000-000000000001` | PASS | `verify-audit-columns.txt` |
| `psql -d spendguard_canonical -f scripts/db/explain-ga-plans.sql` | PASS | `explain-ga-plans.txt` |
