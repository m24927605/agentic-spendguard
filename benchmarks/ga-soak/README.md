# GA Soak Harness

The GA soak harness runs the local demo stack plus the predictor and operations services for a sustained window, writing periodic evidence instead of only a final status.

Local acceptance gate:

```bash
scripts/soak/ga-soak.sh --duration 30m --profile local
```

Release-grade invocation:

```bash
scripts/soak/ga-soak.sh --duration 24h --interval 5m --profile local
```

Evidence is written under `docs/reviews/ga-readiness/GA_07_soak_harness/`:

- `ga_soak_snapshots.jsonl` contains one JSON object per snapshot.
- `ga_soak_summary.json` contains the final pass/fail rollup.
- `ga_soak_baseline.json` pins the first observed container RSS values for growth checks.

The local profile verifies:

- audit chain population plus `verify-chain` via `tests/e2e/verify_audit_columns.py`;
- outbox lag, pending row drain, leader count, and canonical ingest dedup metrics;
- stats aggregator cycle freshness and cache row freshness signals;
- output predictor SVID/mTLS behavior through the HARDEN_08 Rust integration test and Python reference plugin SVID checks;
- container status, health, restart counts, and memory growth.
