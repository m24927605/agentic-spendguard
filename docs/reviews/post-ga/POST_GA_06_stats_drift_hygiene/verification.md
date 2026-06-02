# POST_GA_06 Verification

## Scope

- #157: 24h prediction drift alert cooldown/dedup per `(tenant_id, model, agent_id, prompt_class)`.
- #162: fail-closed guard for NaN/Infinity drift math and alert payload values.

## Command Results

| Gate | Command | Result |
|---|---|---|
| Format | `cargo fmt --manifest-path services/stats_aggregator/Cargo.toml` | PASS |
| Tests | `cargo test --manifest-path services/stats_aggregator/Cargo.toml` | PASS: 32 lib tests, 1 main test, 9 Postgres integration tests, 0 doc tests |
| Build | `cargo build --manifest-path services/stats_aggregator/Cargo.toml` | PASS |
| Helm demo | `helm template charts/spendguard --set chart.profile=demo` | PASS: rendered 1443 lines |
| Helm production | `helm template charts/spendguard -f charts/spendguard/values-production.example.yaml --set chart.profile=production` | PASS: rendered 2157 lines |
| Clean demo | `make demo-down && make demo-up DEMO_MODE=default` | PASS: Step 8 assertions, outbox closure, canonical_events count=5 |
| Demo migration smoke | `docker compose -f deploy/demo/compose.yaml exec -T postgres psql ... prediction_drift_alert_cooldowns` | PASS: table exists, pending columns present, RLS enabled+forced, PK and suppress_until index present, finite z-score checks present |
| Round 1 migration constraint smoke | `docker compose -f deploy/demo/compose.yaml exec -T postgres psql ... pg_get_constraintdef` | PASS: `agent_id` uses `char_length <= 128`; `model` uses `char_length <= 64`; `prompt_class` uses canonical 7-class enum; `last_z_score` rejects `NaN`/`+/-Infinity` |
| Stats daemon smoke | `docker compose -f deploy/demo/compose.yaml up -d --build stats-aggregator && curl /healthz && curl /metrics` | PASS: release image built; health `ok`; metrics include `cycles_total`, `drift_alerts_total`, and `drift_alerts_suppressed_total` |
| In-scope diff check | `git diff --check main..HEAD -- services/stats_aggregator ...` | PASS |

## Test Coverage Added

- In-memory cooldown tests cover same-key suppression, 24h expiry, tenant isolation, model/agent/prompt key separation, and commit-then-timeout retry using the same pending CloudEvent id.
- Postgres integration tests apply migration 0022 and verify same-key suppression, expiry, different-prompt allowance, different-model allowance, different-agent allowance, different-tenant allowance, pending event reuse until `record_emitted`, RLS, canonical 128-character multibyte `agent_id` acceptance, and SQL CHECK rejection for `NaN`, `Infinity`, and `-Infinity`.
- Numeric guard tests cover non-finite inputs, zero stddev, non-finite threshold, non-positive threshold at payload build, and non-finite alert payload rejection.

## Self-Review Notes

- PostgreSQL 16 returns true for `'NaN'::REAL = 'NaN'::REAL`; migration 0022 explicitly rejects `'NaN'::REAL` instead of relying on `x = x`.
- Round 2 reviewer found that pre-append active cooldown reservation could turn append failures into 24h alert loss. Runtime now reserves only a pending retryable CloudEvent before append and records the active 24h cooldown only after durable append or dedupe succeeds.
- Round 1 reviewer found that byte-length constraints could reject canonical multibyte `agent_id` values. Migration 0022 now mirrors canonical `char_length` constraints and enum constraints instead.
- Round 5 reviewer found commit-then-timeout could retry with a fresh CloudEvent id. Staff+ arbitration required an in-slice fix: pending signed CloudEvent proto bytes now persist before append and are reused until canonical_ingest returns `APPENDED` or `DEDUPED`.
