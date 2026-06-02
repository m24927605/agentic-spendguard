# Round 1 Direct Codex Review

Reviewer: codex CLI direct fallback after AIT review orchestration returned `attempt is not reviewable`.

## Findings

1. **Blocker**: `spendguard_output_predictor_rate_limited_total` exported raw `tenant_id` as a Prometheus label, violating the GA metrics inventory and validator high-cardinality/PII rules. The LRU-backed metric counter could also evict and recreate tenant series, making it non-monotonic.
2. **Major**: The per-tenant limiter was process-local but documented/exposed as a service-wide tenant rate. Multi-replica deployments would get approximately `limit * replicas`, and LRU eviction could reset bucket state.
3. **Major**: The API/audit consistency gate was not fully evidenced. The output predictor test verified response echo, but the successful egress proxy consumer still populated `ClaimEstimate.prediction_policy_used` from request policy rather than predictor response.

## Fixes Applied

- Replaced tenant-labeled rate-limit metric samples with a single bounded-label monotonic counter and kept tenant detail in structured logs.
- Replaced limiter LRU tenant state with a bounded `HashMap`; capacity exhaustion now fails closed for new tenants instead of evicting/resetting existing tenant buckets.
- Documented per-pod limiter semantics in the output predictor spec and Helm values.
- Added metrics inventory coverage for `spendguard_output_predictor_rate_limited_total` with no labels.
- Updated egress proxy successful predictor path to use `PredictResponse.prediction_policy_used`, retaining request-policy fallback only for legacy/empty responses.
- Added tests for metric monotonicity, limiter capacity behavior, and egress proxy response-policy propagation.

## Post-Fix Verification

- `cargo test --manifest-path services/output_predictor/Cargo.toml`: PASS, 155 lib tests, 7 main tests, 20 integration tests, 0 doctests.
- `cargo test --manifest-path services/egress_proxy/Cargo.toml`: PASS, 123 main tests, 1 decision integration marker, 92 multi-provider tests.
- `cargo build --manifest-path services/output_predictor/Cargo.toml`: PASS.
- `cargo build --manifest-path services/egress_proxy/Cargo.toml`: PASS with existing warnings.
- `scripts/observability/validate-dashboard-metrics.sh`: PASS, 19 metrics, 19 expressions.
- `helm template spendguard charts/spendguard --set chart.profile=demo`: PASS, 1445 lines.
- `helm template spendguard charts/spendguard --set chart.profile=production -f charts/spendguard/values-production.example.yaml`: PASS, 2159 lines.
- `docker compose -f deploy/demo/compose.yaml up -d --build output-predictor`: PASS.
- `curl -fsS http://localhost:9100/healthz`: PASS, `ok`.
- `curl -fsS http://localhost:9100/metrics | rg "spendguard_output_predictor_(rate_limited_total|predict_total)"`: PASS; rate-limit metric has no labels.
- `make demo-down && make demo-up DEMO_MODE=default`: PASS.
- `git diff --check`: PASS.
