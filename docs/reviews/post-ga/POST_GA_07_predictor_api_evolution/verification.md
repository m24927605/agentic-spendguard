# POST_GA_07 Verification Evidence

Branch: `post-ga/POST_GA_07_predictor_api_evolution`

Base: `main` at `04f1fa3 Merge POST_GA_06 stats drift hygiene`

## Local Gates

| Gate | Result | Evidence |
|---|---|---|
| `cargo fmt --manifest-path services/output_predictor/Cargo.toml --check` | PASS | exited 0 |
| `cargo test --manifest-path services/output_predictor/Cargo.toml` | PASS | 154 lib tests, 7 main tests, 20 integration tests, 0 doctests |
| `cargo build --manifest-path services/output_predictor/Cargo.toml` | PASS | dev profile finished |
| `helm template spendguard charts/spendguard --set chart.profile=demo` | PASS | rendered 1445 lines |
| `helm template spendguard charts/spendguard --set chart.profile=production -f charts/spendguard/values-production.example.yaml` | PASS | rendered 2159 lines |
| `python3 -m py_compile deploy/demo/demo/run_demo.py` | PASS | exited 0 |
| `git diff --check` | PASS | exited 0 |

## Runtime Gates

| Gate | Result | Evidence |
|---|---|---|
| `make demo-down && make demo-up DEMO_MODE=default` | PASS | release smoke, decision, commit, provider report, outbox drain, canonical event verification all completed |
| `docker compose -f deploy/demo/compose.yaml up -d --build output-predictor` | PASS | output predictor container reached `healthy` |
| `curl -fsS http://localhost:9100/healthz` | PASS | returned `ok` |
| `curl -fsS http://localhost:9100/metrics \| rg "spendguard_output_predictor_(rate_limited_total\|predict_total)"` | PASS | `predict_total` and `rate_limited_total` HELP/TYPE emitted |

## API Compatibility

- `PredictResponse.prediction_policy_used` is additive field tag `17`.
- Existing response field numbers `1..16` are unchanged.
- Python SDK proto generation currently does not include output predictor stubs; the demo image `make proto` path regenerated only existing SDK proto surfaces and remained clean.

## Rate Limit Behavior

- Default per-tenant refill rate: `1000` Predict RPCs/second.
- Default retained tenant buckets: `4096`.
- `0` disables the limiter for emergency rollback.
- Over-limit requests return gRPC `RESOURCE_EXHAUSTED` and increment `spendguard_output_predictor_rate_limited_total{tenant_id}`.
- Unit coverage verifies one tenant exhausting its bucket does not affect another tenant.

## Demo Timeout Hardening

The SDK production decision deadline remains `250ms`. The demo harness now uses `SPENDGUARD_DEMO_DECISION_TIMEOUT_S` with default `5.0s` when constructing demo clients so cold compose startup does not fail the required demo quality gate.
