# POST_GA_08 Verification Evidence

Branch: `post-ga/POST_GA_08_db_index_and_rls_polish`

Base: `main` at `3c2a240 Merge POST_GA_07 predictor API evolution`

## Local Gates

| Gate | Result | Evidence |
|---|---|---|
| `scripts/release/verify-migration-inventory.sh` | PASS | inventory verified after adding ledger `0054` and canonical `0023` |
| `scripts/verify-migrations-postgres16.sh` | PASS | Postgres 16.14 container applied ledger/canonical/control-plane migrations and all POST_GA_08 smoke checks |
| `cargo fmt --manifest-path services/stats_aggregator/Cargo.toml --check` | PASS | exited 0 |
| `cargo build --manifest-path services/stats_aggregator/Cargo.toml` | PASS | dev profile finished |
| `cargo test --manifest-path services/stats_aggregator/Cargo.toml` | PASS | 32 lib tests, 1 main test, 10 Postgres integration tests, 0 doctests |
| `helm template spendguard charts/spendguard --set chart.profile=demo` | PASS | rendered 1445 lines |
| `helm template spendguard charts/spendguard --set chart.profile=production -f charts/spendguard/values-production.example.yaml` | PASS | rendered 2159 lines |
| `git diff --check` | PASS | exited 0 |

## Runtime Gate

| Gate | Result | Evidence |
|---|---|---|
| `make demo-down && make demo-up DEMO_MODE=default` | PASS | clean-volume release smoke, decision, commit, provider report, outbox drain, and canonical event verification all completed |

## POST_GA_08 Issue Evidence

| Issue | Evidence |
|---|---|
| #146 | `0054_tokenizer_t1_samples_public_select_revoke.sql` revokes parent/current-partition SELECT from PUBLIC; migration smoke `migration-smoke-ledger-tokenizer-public-revoke.txt` proves no PUBLIC SELECT grant remains. |
| #163 | `0023_cache_rls_no_nil_sentinel.sql` replaces nil UUID sentinel fallback with NULL comparison for `output_distribution_cache` and `run_length_distribution_cache`; integration test `rls_missing_tenant_setting_does_not_match_nil_uuid_rows` proves missing tenant setting cannot read nil-tenant rows. |
| #164 | `docs/operations/runbooks/stats-aggregator-advisory-lock-stall.md` documents detection, TCP keepalive checks, safe `pg_terminate_backend`, rollback, and evidence capture for stale advisory-lock sessions. |
| #166 | `scripts/db/explain-post-ga-08-cache-index.sql` and `migration-smoke-canonical-output-cache-index-plan.txt` prove `output_distribution_cache_freshness_idx` is used for stale range scans and `max(computed_at)` SLO probes; hot lookup remains primary-key backed. |

## Evidence Files

- `migration-smoke-postgres-version.txt`
- `migration-smoke-ledger-tokenizer-public-revoke.txt`
- `migration-smoke-canonical-rls-no-nil-sentinel.txt`
- `migration-smoke-canonical-output-cache-index-plan.txt`
- `migration-smoke-*-smoke.txt`
