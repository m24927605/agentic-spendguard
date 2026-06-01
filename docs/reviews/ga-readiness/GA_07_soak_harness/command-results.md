# GA_07 Command Results

Date: 2026-06-01

| Gate | Result | Evidence |
|---|---|---|
| `scripts/soak/ga-soak.sh --duration 30m --profile local` | PENDING | Writes `ga_soak_summary.json`, `ga_soak_snapshots.jsonl`, and `ga_soak_baseline.json` |
| `cargo test --manifest-path services/output_predictor/Cargo.toml --test plugin_svid_mtls -- --nocapture` | PENDING | Run inside the soak harness before sustained snapshots |
| `python3 -m pytest contrib/output_predictor_template/conformance_test.py -q -k 'client_svid'` | PENDING | Run inside the soak harness before sustained snapshots |
| `tests/e2e/verify_audit_columns.py --tenant 00000000-0000-4000-8000-000000000001` | PENDING | Run on every soak snapshot |
| `helm template spendguard charts/spendguard --set chart.profile=demo` | PENDING | Standard GA chart gate |
| `helm template spendguard charts/spendguard --set chart.profile=production -f scripts/helm-validate-test-values.yaml` | PENDING | Standard GA chart gate |
| `git diff --check` | PENDING | Whitespace gate |
