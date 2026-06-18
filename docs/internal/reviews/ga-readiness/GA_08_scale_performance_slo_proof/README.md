# GA_08 Scale/Performance Evidence

This directory contains evidence produced by `benchmarks/ga-load/run.sh`.

The local compose scenario is a real-stack smoke and DB-plan gate. It is not
the Contract §14 latency certification gate. Contract §14 p99 certification is
owned by `spendguard-predictor-upgrade-benchmarks`, which fails when
SpendGuard decision p99 exceeds 50,000us.

Required merge evidence:

- `load-results.json`
- `ga_load_summary.json`
- `command-results.md`
- `verify-audit-columns.txt`
- `explain-ga-plans.txt`

`ga_load_summary.json.commit_sha` records the clean source commit under test. The evidence commit that stores generated evidence necessarily follows the run and must not change the load harness or DB plan gate.
