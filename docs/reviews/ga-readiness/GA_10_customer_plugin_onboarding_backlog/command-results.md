# GA 10 Command Results

Date: 2026-06-01

Branch: `ga/GA_10_customer_plugin_onboarding_backlog`

| Command | Result | Notes |
|---|---|---|
| `scripts/ga/validate-ga10.sh` | PASS | Customer docs, taxonomy modes, template README link, and issue #85-#177 coverage validated. |
| `python3 -m pytest contrib/output_predictor_template/conformance_test.py -q` | PASS | 70 passed in 8.23s. Python emitted a local LibreSSL warning from urllib3; tests passed. |
| `gh issue close 106 107 128 138 142 144 153 155 170` | PASS | Closed only the resolved/duplicate/historical GA_10 closure set. See `issue-closures.md`. |
| `helm template spendguard charts/spendguard --set chart.profile=demo` | PASS | Rendered cleanly to `/tmp/ga10-helm-demo.yaml`. |
| `helm template spendguard charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production` | PASS | Rendered cleanly to `/tmp/ga10-helm-production.yaml`. |
| `make demo-up DEMO_MODE=plugin_c_synthetic` | PASS | Strategy C breaker-open regression passed, HARDEN_08 per-tenant SVID tests passed, and Python SVID subset passed. |

## Current Evidence

- Customer onboarding docs exist under `docs/customer/`.
- Template README links the certification path and repeats SVID/mTLS,
  timeout, retry, circuit-breaker, and fallback expectations.
- Backlog triage report covers every number in #85-#177, including
  #88 and #89 as not present in GitHub results.
