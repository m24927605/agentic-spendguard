# POST_GA_06 Round 3 Fixes

Reviewer: codex CLI direct adversarial fallback after AIT nested-wrapper failure.

## Findings Fixed

- Minor: numeric guard skips were not counted in
  `spendguard_stats_aggregator_drift_alerts_suppressed_total` even though the
  metric help text includes numeric safety guards.
- Minor: the Postgres cooldown key test covered tenant and prompt-class
  independence, but not model and agent-id independence.

## Implementation

- Added `numeric_guard_suppression_reason` in
  `services/stats_aggregator/src/drift_detector.rs`.
- `detect_and_emit` now increments `suppressed` and logs the reason when a
  non-finite value, non-positive baseline stddev, non-finite computed z-score,
  or invalid threshold suppresses alert emission.
- Expanded `drift_alert_cooldown_postgres_is_key_and_tenant_scoped` to verify
  independent cooldown behavior for all key dimensions:
  `(tenant_id, model, agent_id, prompt_class)`.

## Verification

- `cargo fmt --manifest-path services/stats_aggregator/Cargo.toml`
- `cargo test --manifest-path services/stats_aggregator/Cargo.toml`
  - 30 lib tests passed
  - 1 main test passed
  - 7 Postgres integration tests passed
