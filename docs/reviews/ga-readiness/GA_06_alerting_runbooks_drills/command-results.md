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
| `ait run --adapter codex --review-mode adversarial ...` | CLI FAIL | Local AIT rejected `--review-mode`; direct codex CLI review used as the recorded reviewer fallback |
| Codex CLI adversarial review R1 | FINDINGS FIXED | Fixed absent-series no-leader alerting, stable PrometheusRule name, and strict `>60` plus 5m hold drill semantics |
| Codex CLI adversarial review R2 | PASS | Reviewer found no actionable regressions in alert rules, validator, runbooks, or drill |

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

Notes:

- AIT CLI compatibility: `ait run --adapter codex --review-mode adversarial ...` failed locally with `unrecognized arguments: --review-mode`; direct codex CLI review was run with `codex review --base main` and recorded per round.
- R1 findings were fixed in-slice. R2 was clean, so no Staff+ arbitration was needed.
