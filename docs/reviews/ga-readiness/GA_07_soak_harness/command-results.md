# GA_07 Command Results

Date: 2026-06-01

| Gate | Result | Evidence |
|---|---|---|
| `scripts/soak/ga-soak.sh --duration 30m --profile local` | PASS | `ga_soak_summary.json`: result `pass`, duration `1800`, snapshots `27`, started `2026-06-01T00:27:51Z`, finished `2026-06-01T00:58:12Z` |
| `cargo test --manifest-path services/output_predictor/Cargo.toml --test plugin_svid_mtls -- --nocapture` | PASS | Run inside the soak harness before sustained snapshots; 4 tests passed |
| `python3 -m pytest contrib/output_predictor_template/conformance_test.py -q -k 'client_svid'` | PASS | Run inside the soak harness before sustained snapshots; 5 tests passed, 65 deselected |
| `python3 tests/e2e/verify_audit_columns.py --tenant 00000000-0000-4000-8000-000000000001` | PASS | Run on every soak snapshot; last snapshot verify status `0`, verify-chain GREEN |
| `helm template spendguard charts/spendguard --set chart.profile=demo` | PASS | Rendered `/tmp/ga07-helm-demo.yaml`, 1441 lines |
| `helm template spendguard charts/spendguard --set chart.profile=production -f scripts/helm-validate-test-values.yaml` | PASS | Rendered `/tmp/ga07-helm-prod.yaml`, 1534 lines |
| `git diff --check` | PASS | Whitespace gate clean before the 30m run; rerun after evidence update before review |

Last soak snapshot:

| Field | Value |
|---|---:|
| `elapsed_seconds` | 1814 |
| `canonical_events` | 5 |
| `pending_forward_rows` | 0 |
| `outbox_lag_metric_seconds` | 0 |
| `outbox_leader_count` | 1 |
| `stats_cycles_total` | 31 |
| `stats_errors_total` | 0 |
| `last_cycle_age_seconds` | 35 |
| `svid_probe_status` | 0 |
| `container_count` | 10 |
| `failures` | 0 |
