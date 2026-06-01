# GA_08 Scale/Performance Evidence

This directory contains evidence produced by `benchmarks/ga-load/run.sh`.

Required merge evidence:

- `load-results.json`
- `ga_load_summary.json`
- `command-results.md`
- `verify-audit-columns.txt`
- `explain-ga-plans.txt`

`ga_load_summary.json.commit_sha` records the clean source commit under test. The evidence commit that stores generated evidence necessarily follows the run and must not change the load harness or DB plan gate.
