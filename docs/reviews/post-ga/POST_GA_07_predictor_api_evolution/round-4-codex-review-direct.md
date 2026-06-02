# Round 4 Direct Codex Review

Reviewer: codex CLI direct fallback after AIT review orchestration returned `attempt is not reviewable`.

## Findings

1. **Major**: Final HEAD still carried the old labeled-metric sample literal in the output predictor metrics unit test and verification evidence. Runtime metrics were no-label, but the code/evidence text violated the final stale-wording invariant.

## Fixes Applied

- Replaced the metrics unit test's negative literal check with shape parsing: exactly one `rate_limited_total` sample, metric name followed directly by a value, and no extra fields.
- Updated POST_GA_07 verification evidence to describe the live sample shape without embedding the old labeled sample literal.

## Post-Fix Verification

- `cargo test --manifest-path services/output_predictor/Cargo.toml render_metrics_contains_known_names`: PASS.
- `cargo fmt --manifest-path services/output_predictor/Cargo.toml --check`: PASS after formatting.
- Legacy labeled-metric wording grep across output predictor source, POST_GA_07 review artifacts, and output predictor spec: PASS, no matches.
- `git diff --check`: PASS.
