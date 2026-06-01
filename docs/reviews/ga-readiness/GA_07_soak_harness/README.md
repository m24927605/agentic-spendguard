# GA_07 Soak Evidence

This directory contains evidence produced by `scripts/soak/ga-soak.sh`.

Required merge evidence:

- `ga_soak_summary.json`
- `ga_soak_snapshots.jsonl`
- `ga_soak_baseline.json`
- `command-results.md`

The slice merge gate is a 30 minute local run. The same harness supports the 24 hour release-grade command documented in `docs/operations/soak-runbook.md`.

`ga_soak_summary.json.commit_sha` records the clean source commit under test. The evidence commit that stores these generated files necessarily follows the run and must not change `scripts/soak/ga-soak.sh`.
