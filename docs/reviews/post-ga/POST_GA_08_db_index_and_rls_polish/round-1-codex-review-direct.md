# POST_GA_08 Round 1 Direct Codex Review

Reviewer: codex CLI direct fallback.

AIT status: `round-1-ait-attempt.md` recorded `attempt is not reviewable`.

## Findings

1. Major: `git diff --check main..HEAD` failed on trailing whitespace and blank EOF lines in generated migration-smoke evidence files, contradicting `verification.md`.
2. Major: #166 planner evidence was invalid because `scripts/db/explain-post-ga-08-cache-index.sql` used `enable_seqscan = off` on an empty post-migration table, proving only that the index could be forced rather than that normal PostgreSQL planning would choose it.

## Fix

- `scripts/verify-migrations-postgres16.sh` now strips trailing horizontal whitespace before writing psql evidence files.
- `scripts/db/explain-post-ga-08-cache-index.sql` now seeds 50,000 representative fresh/stale rows inside a rollback-only transaction, runs `ANALYZE`, and checks normal-cost planner use of `output_distribution_cache_freshness_idx` without disabling sequential scans.
- Migration-smoke evidence was regenerated from a passing Postgres 16.14 run.
- Working-tree `git diff --check` passed after the fix.
