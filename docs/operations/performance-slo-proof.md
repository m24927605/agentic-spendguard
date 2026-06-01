# Performance SLO Proof

GA_08 uses `benchmarks/ga-load/run.sh` as the local proof for predictor-upgrade scale behavior.

## Local Gate

```bash
benchmarks/ga-load/run.sh --scenario benchmarks/ga-load/scenarios/local-100-tenants.yaml
```

The command resets and boots the demo compose stack with:

- tokenizer
- output_predictor
- run_cost_projector
- sidecar
- ledger
- canonical_ingest
- stats_aggregator
- outbox_forwarder

The load driver runs inside the demo adapter container so the sidecar UDS and compose DNS paths are the same runtime paths used by demos. Every operation calls tokenizer and output_predictor, then submits the resulting prediction metadata through the Python SDK to the sidecar, which reserves, publishes, emits the LLM post event, and forwards audit rows.

The local compose stack does not mount a customer Strategy C plugin. To keep the audit-column population gate meaningful, the driver fills the Strategy C mirror with a conservative synthetic value while leaving `prediction_strategy_used` on the real output_predictor response. This exercises canonical storage, verification, and calibration deltas without claiming a customer plugin SLO.

## Scenario Semantics

The local compose topology has exactly one certified tenant identity. The `local-100-tenants` scenario therefore represents 100 logical customer workloads under that certified demo tenant through distinct run IDs, agent IDs, providers, models, and prompt classes. The harness must not fabricate tenant assertions because sidecar tenant assertion rejection is a security invariant.

## Evidence

The harness writes:

- `load-results.json`: operation counts, cardinality, latency p50/p95/p99/max, service metric counters, and driver failures
- `ga_load_summary.json`: merge-gate summary with commit, branch, git cleanliness, audit deltas, outbox drain status, verify-chain status, and DB plan status
- `command-results.md`: human-readable gate table
- `verify-audit-columns.txt`: audit integrity probe output
- `explain-ga-plans.txt`: canonical DB plan-gate output

## DB Plan Gate

Run the plan gate directly when investigating database changes:

```bash
docker compose -f deploy/demo/compose.yaml exec -T postgres \
  psql -U spendguard -d spendguard_canonical -v ON_ERROR_STOP=1 \
  < scripts/db/explain-ga-plans.sql
```

The SQL validates required indexes and rejects sequential scans over GA production tables for:

- output distribution hot lookup
- run-length cache hot lookup
- output distribution aggregation
- run-length aggregation
- decision/outcome joins

The script disables sequential scans while planning because the local demo database is intentionally small. This is an index-viability gate, not a cloud-capacity certification.

## Release Use

For external release qualification, run the same command against a larger cluster-specific scenario file and archive the evidence directory with the release bundle. Do not lower SLO thresholds in the scenario file without Staff+ approval.
