# GA_06 Command Results

Date: 2026-05-31

| Gate | Result | Notes |
|---|---|---|
| `scripts/observability/validate-alert-runbooks.sh` | PASS | 10 alerts, 10 runbooks, 1 drill, 10 metrics |
| Prometheus rules YAML parse | PASS | Parsed as `monitoring.coreos.com/v1` `PrometheusRule`; 10 alerts |
| `helm template spendguard charts/spendguard --set chart.profile=demo` | PASS | Rendered 1441 lines |
| `helm template spendguard charts/spendguard --set chart.profile=production -f scripts/helm-validate-test-values.yaml` | PASS | Rendered 1534 lines |
| `tests/e2e/outbox_lag_recovery.sh` | PASS | Ran default demo, reproduced outbox lag above the alert threshold for the `for` duration, recovered to zero pending rows |
| `make demo-up DEMO_MODE=default` | PASS | Exercised inside `tests/e2e/outbox_lag_recovery.sh` |
| `git diff --check` | PASS | No whitespace errors |
| Cargo build/test | N/A | No Rust service code changed in GA_06 |

## Drill Evidence

`outbox_lag_recovery.json` recorded:

| Field | Value |
|---|---|
| `result` | `pass` |
| `pending_during_outage` | `1` |
| `lag_metric_during_outage` | `363` |
| `alert_predicate_hold_seconds` | `300` |
| `pending_after_recovery` | `0` |
| `lag_metric_after_recovery` | `0` |
