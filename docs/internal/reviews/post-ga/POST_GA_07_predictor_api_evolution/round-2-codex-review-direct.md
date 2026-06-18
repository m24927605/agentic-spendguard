# Round 2 Direct Codex Review

Reviewer: codex CLI direct fallback after AIT review orchestration returned `attempt is not reviewable`.

## Findings

1. **Major**: The output predictor service spec and config comments still described the `rate_limited_total` counter as carrying tenant detail, contradicting the round-1 implementation that exports `spendguard_output_predictor_rate_limited_total` without labels.

## Fixes Applied

- Updated `docs/output-predictor-service-spec-v1alpha1.md` to state that over-limit Predict requests log tenant id in structured logs and increment a no-label monotonic counter.
- Updated the §9 failure-mode row to remove stale tenant-detail-in-Prometheus wording.
- Updated `services/output_predictor/src/config.rs` to describe bounded limiter state without implying tenant detail in metrics.
- Updated POST_GA_07 verification/review evidence to consistently call `rate_limited_total` a no-label counter.

## Post-Fix Verification

- Legacy tenant-metric wording grep across the affected spec/config/evidence files: PASS, no stale matches.
- `cargo fmt --manifest-path services/output_predictor/Cargo.toml --check`: PASS.
- `cargo test --manifest-path services/output_predictor/Cargo.toml`: PASS, 155 lib tests, 7 main tests, 20 integration tests, 0 doctests.
- `git diff --check`: PASS.
