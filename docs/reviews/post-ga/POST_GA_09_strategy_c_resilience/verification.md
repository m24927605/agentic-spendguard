# POST_GA_09 Verification Evidence

Branch: `post-ga/POST_GA_09_strategy_c_resilience`

Base: `main` at `d524793 Merge POST_GA_08 DB index and RLS polish`

## Local Gates

| Gate | Result | Evidence |
|---|---|---|
| `cargo fmt --manifest-path services/output_predictor/Cargo.toml --check` | PASS | exited 0 after formatting |
| `cargo build --manifest-path services/output_predictor/Cargo.toml` | PASS | dev profile finished |
| `cargo test --manifest-path services/output_predictor/Cargo.toml` | PASS | 163 lib tests, 7 main tests, 20 integration tests, 0 doctests |
| `cargo fmt --manifest-path services/control_plane/Cargo.toml --check` | PASS | exited 0 after formatting |
| `cargo build --manifest-path services/control_plane/Cargo.toml` | PASS | dev profile finished; existing auth dead-code warnings only |
| `cargo test --manifest-path services/control_plane/Cargo.toml` | PASS | 55 tests passed; existing auth dead-code warnings only |
| `helm template spendguard charts/spendguard --set chart.profile=demo` | PASS | rendered 1445 lines |
| `helm template spendguard charts/spendguard --set chart.profile=production -f charts/spendguard/values-production.example.yaml` | PASS | rendered 2159 lines |

## Runtime Gate

| Gate | Result | Evidence |
|---|---|---|
| `make demo-down && make demo-up DEMO_MODE=plugin_c_synthetic` | PASS | Strategy C breaker-open regression, per-tenant SVID mTLS tests, and plugin template pytest all passed; final line: `plugin_c_synthetic PASS` |

## POST_GA_09 Issue Evidence

| Issue | Evidence |
|---|---|
| #172 | `force_reset` audit data now includes `operation`, `tenant_id`, `plugin_endpoint_id`, `previous_health_status`, `new_health_status`, `reset_at`, bounded `reason`, `reason_length`, and an explicit status-only `effect` that does not claim direct output_predictor breaker mutation. |
| #173 | `validate_force_reset_reason` trims, requires non-empty, and rejects reason strings over `MAX_FORCE_RESET_REASON_LEN=1024` before audit/logging. |
| #174 | `EndpointCache` now uses tenant-scoped reload locks plus 1s miss/DB-error reload-result backoff so stale/missing cache reloads singleflight per tenant without serializing unrelated tenants or queueing one DB hit per waiter. |
| #175 | `EndpointCache` serves enabled stale endpoint snapshots for at most 300s on control-plane DB errors, falls back after the bound, and does not resurrect `enabled=false` stale endpoints. |
| #176 | `OutputPredictorSvc::predict` rejects `decision_id` and `prompt_class_fingerprint` over 128 bytes before cache, DB, or plugin work. |
