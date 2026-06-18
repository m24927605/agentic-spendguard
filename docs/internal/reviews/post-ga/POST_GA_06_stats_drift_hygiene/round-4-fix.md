# POST_GA_06 Round 4 Fixes

Reviewer: codex CLI direct adversarial fallback after AIT command failure.

## Findings Fixed

- Minor: the new cooldown table lacked adversarial RLS coverage for missing
  or mismatched `app.current_tenant_id`.
- Minor: there was no regression test for the documented failure mode where
  immutable append succeeds but cooldown recording fails afterward.

## Implementation

- Added `drift_alert_cooldown_postgres_rls_blocks_missing_or_mismatched_tenant`
  in `services/stats_aggregator/tests/cycle_e2e_postgres.rs`.
  The test uses the non-owner app role, seeds a tenant A cooldown, then proves
  missing tenant context and tenant B context cannot read or write tenant A.
- Added `RecordFailingCooldown` and
  `detect_and_emit_counts_durable_append_when_cooldown_record_fails` in
  `services/stats_aggregator/src/drift_detector.rs`.
  The test verifies a successful durable append is counted as emitted even when
  post-append cooldown recording fails.

## Verification

- `cargo fmt --manifest-path services/stats_aggregator/Cargo.toml`
- `cargo test --manifest-path services/stats_aggregator/Cargo.toml`
  - 31 lib tests passed
  - 1 main test passed
  - 8 Postgres integration tests passed
