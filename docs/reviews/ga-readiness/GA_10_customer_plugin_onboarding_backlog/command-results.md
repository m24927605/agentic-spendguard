# GA 10 Command Results

Date: 2026-06-01

Branch: `ga/GA_10_customer_plugin_onboarding_backlog`

| Command | Result | Notes |
|---|---|---|
| `scripts/ga/validate-ga10.sh` | PASS | Customer docs, live API path, `client_cert_id`, client SVID evidence path, taxonomy modes, template README link, issue #85-#177 coverage, and named post-GA slice consistency validated. |
| `cargo build && cargo test` in `services/output_predictor` | PASS | Build passed; test suite passed after routing SVID/URI-SAN/client-cert `PERMISSION_DENIED` identity failures to `tls_error` (`151 + 7 + 4 + 7 + 1 + 4 + 4` tests). |
| `python3 -m pytest contrib/output_predictor_template/conformance_test.py -q` | PASS | 70 passed in 26.08s after R3 fixes. Python emitted a local LibreSSL warning from urllib3; tests passed. |
| `for n in 106 107 128 138 142 144 153 155 170; do gh issue close "$n" --repo m24927605/agentic-spendguard; done` | PASS | Closed only the resolved/duplicate/historical GA_10 closure set. See `issue-closures.md`. |
| `helm template spendguard charts/spendguard --set chart.profile=demo` | PASS | Rendered cleanly to `/tmp/ga10-helm-demo.yaml`. |
| `helm template spendguard charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production` | PASS | Rendered cleanly to `/tmp/ga10-helm-production.yaml`. |
| `make demo-up DEMO_MODE=plugin_c_synthetic` | PASS | Strategy C breaker-open regression passed, HARDEN_08 per-tenant SVID tests passed, and Python SVID subset passed. |
| `codex review --base main` R4 | FINDING FIXED | P1 checklist gap closed: certification now requires explicit reference-image `command`/`args` override or runtime proof that `--insecure` was not used. |

## Current Evidence

- Customer onboarding docs exist under `docs/customer/`.
- Template README links the certification path and repeats SVID/mTLS,
  timeout, retry, circuit-breaker, and fallback expectations.
- Backlog triage report covers every number in #85-#177, including
  #88 and #89 as not present in GitHub results.
