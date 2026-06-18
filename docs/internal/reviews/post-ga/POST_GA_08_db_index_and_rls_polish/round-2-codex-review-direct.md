# POST_GA_08 Round 2 Direct Codex Review

Reviewer: codex CLI direct fallback.

AIT status: `round-2-ait-attempt.md` recorded `attempt is not reviewable`.

## Finding

1. Major: `docs/operations/runbooks/stats-aggregator-advisory-lock-stall.md` used advisory lock id `5994358719602389587`, but the shipped Rust constant is `0x5350_4441_4747_5253` = `6003373350444290643` in `services/stats_aggregator/src/aggregation.rs`. The runbook `pg_locks` query would target the wrong advisory lock and fail #164 detection.

## Fix

- The runbook now uses `6003373350444290643` in both the `pg_locks` query and explanatory lock-id text.
- Round 2 reviewer also confirmed `git diff --check main..HEAD` and `scripts/release/verify-migration-inventory.sh` passed before this fix.
