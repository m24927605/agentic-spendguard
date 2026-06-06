# D15 — Acceptance Gates

Per build plan §3, every gate listed here must be **100% feasible** at slice-spec time: runnable in the current repo state, no third-party action required, reproducible by the `superpowers:code-reviewer` skill without privileged Manus credentials.

Per build plan §3 (importer-specific clause): feasibility = "billing-importer endpoint is testable against a vendor-staged fixture, even if the live admin API is gated." The headline gate is **fixture-driven**, not live.

## 1. Repository-state gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A1.1` | `services/importer_manus/Cargo.toml` exists; pkg name `spendguard-importer-manus` | `cargo metadata --format-version 1 \| jq -e '.packages[] \| select(.name == "spendguard-importer-manus")'` |
| `A1.2` | Crate is in the workspace | `grep -qE 'services/importer_manus' Cargo.toml` |
| `A1.3` | `services/importer_manus/src/lib.rs` re-exports `record`, `pricing`, `fixture`, `audit`, `error` modules | `grep -qE 'pub mod record\|pub mod pricing\|pub mod fixture\|pub mod audit\|pub mod error' services/importer_manus/src/lib.rs` |
| `A1.4` | `services/importer_manus/assets/price_table.toml` exists with `team_plan`, `enterprise`, `enterprise_byok` keys | `grep -qE '\[tiers\.team_plan\]\|\[tiers\.enterprise\]\|\[tiers\.enterprise_byok\]' services/importer_manus/assets/price_table.toml` (all three matched) |
| `A1.5` | Fixture `services/importer_manus/tests/fixtures/manus_usage.json` committed with 8 sessions | `jq '.sessions \| length' services/importer_manus/tests/fixtures/manus_usage.json` equals `8` |
| `A1.6` | Fixture `PROVENANCE.md` exists with redaction script SHA-256 pinned | `grep -qE 'sha256:[0-9a-f]{64}' services/importer_manus/tests/fixtures/PROVENANCE.md` |
| `A1.7` | Migration `0047_audit_outbox_extend_reservation_source.sql` exists | `test -f services/canonical_ingest/migrations/0047_audit_outbox_extend_reservation_source.sql` |
| `A1.8` | Migration `0048_audit_outbox_extend_import_source.sql` exists | `test -f services/canonical_ingest/migrations/0048_audit_outbox_extend_import_source.sql` |
| `A1.9` | Down-migrations exist for both | `test -f services/canonical_ingest/migrations/down/0047_audit_outbox_extend_reservation_source_down.sql && test -f services/canonical_ingest/migrations/down/0048_audit_outbox_extend_import_source_down.sql` |
| `A1.10` | `migration_inventory.toml` lists 0047 + 0048 with SHA pins | `grep -qE '0047_audit_outbox_extend_reservation_source' services/canonical_ingest/migration_inventory.toml && grep -qE '0048_audit_outbox_extend_import_source' services/canonical_ingest/migration_inventory.toml` |
| `A1.11` | `docs/site-v2/src/content/docs/integrations/manus-importer.md` exists | `test -f docs/site-v2/src/content/docs/integrations/manus-importer.md` |
| `A1.12` | `README.md` `## Adapter integrations` table includes a "Manus importer" row | `grep -q 'Manus importer' README.md` |
| `A1.13` | CloudEvent type `spendguard.audit.import.manus_credit` registered in `outbox_forwarder` | `grep -qE 'spendguard\.audit\.import\.manus_credit' services/outbox_forwarder/src/cloudevent_types.rs` |
| `A1.14` | Demo verifier SQL `deploy/demo/verify_step_import_manus.sql` committed | `test -f deploy/demo/verify_step_import_manus.sql` |
| `A1.15` | Demo Makefile target `demo-import-manus-fixture` defined | `grep -qE 'demo-import-manus-fixture:' deploy/demo/Makefile` |

## 2. Build gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A2.1` | Workspace builds | `cargo build --workspace --locked` exits 0 |
| `A2.2` | Importer default-features build clean | `cargo build -p spendguard-importer-manus --locked` exits 0 |
| `A2.3` | Importer `live` build clean | `cargo build -p spendguard-importer-manus --features live --locked` exits 0 |
| `A2.4` | Default build pulls NO HTTP client | `cargo tree -p spendguard-importer-manus -e=normal \| grep -vE '^(spendguard-importer-manus\|spendguard-common\|serde\|serde_json\|toml\|chrono\|anyhow\|thiserror\|tracing\|tokio)' \| grep -E 'reqwest\|hyper-tls\|hyper-rustls'` returns no matches |
| `A2.5` | `live`-features build DOES pull reqwest | `cargo tree -p spendguard-importer-manus --features live -e=normal \| grep -qE 'reqwest'` |
| `A2.6` | No new warnings | `cargo build -p spendguard-importer-manus --all-features -- -D warnings` exits 0 |
| `A2.7` | Clippy clean | `cargo clippy -p spendguard-importer-manus --all-targets --all-features -- -D warnings` exits 0 |
| `A2.8` | `cargo deny check` passes | `cargo deny check` exits 0 |
| `A2.9` | `publish = false` in Cargo.toml | `grep -qE '^publish *= *false' services/importer_manus/Cargo.toml` |

## 3. Unit-test gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A3.1` | All pricing unit tests green | `cargo test -p spendguard-importer-manus --lib pricing::tests` exits 0 |
| `A3.2` | All fixture-loader unit tests green | `cargo test -p spendguard-importer-manus --lib fixture::tests` exits 0 |
| `A3.3` | All audit-row unit tests green | `cargo test -p spendguard-importer-manus --lib audit::tests` exits 0 |
| `A3.4` | Property test: 200 random valid `ImportRecord`s → `amount_micro_usd >= 0` | `cargo test -p spendguard-importer-manus --lib audit_row_amount_never_negative` exits 0 |
| `A3.5` | Embedded price table loads without panic | `cargo test -p spendguard-importer-manus --lib price_table_load_embedded_succeeds` exits 0 |
| `A3.6` | `live::from_env` rejects missing/empty `MANUS_API_TOKEN` | `cargo test -p spendguard-importer-manus --features live --lib live::tests::from_env_returns_missing_token_when_unset live::tests::from_env_returns_missing_token_when_empty` exits 0 |

## 4. Contract-test gates (real Postgres)

| ID | Gate | Verification command |
|----|------|----------------------|
| `A4.1` | Audit row round-trips through migration 0047 CHECK | `cargo test -p spendguard-importer-manus --test contract audit_row_round_trips_through_migration_0047` exits 0 |
| `A4.2` | Audit row round-trips through migration 0048 CHECK | `cargo test -p spendguard-importer-manus --test contract audit_row_round_trips_through_migration_0048` exits 0 |
| `A4.3` | Invalid `reservation_source` rejected by 0047 CHECK | `cargo test -p spendguard-importer-manus --test contract audit_row_with_invalid_reservation_source_rejected` exits 0 |
| `A4.4` | Invalid `import_source` rejected by 0048 CHECK | `cargo test -p spendguard-importer-manus --test contract audit_row_with_invalid_import_source_rejected` exits 0 |
| `A4.5` | Partial index `idx_audit_outbox_import_manus` present with correct predicate | `cargo test -p spendguard-importer-manus --test contract partial_index_idx_audit_outbox_import_manus_exists` exits 0 |
| `A4.6` | Dedupe key prevents double-import | `cargo test -p spendguard-importer-manus --test contract dedupe_key_uniqueness_prevents_double_import` exits 0 |

## 5. Fixture-driven integration gate (PRIMARY HEADLINE)

This is the merge-blocking gate. Maps 1:1 to the deliverable prompt's acceptance: *fixture-driven test imports recorded `manus_usage.json` and emits correct synthetic audit events with `reservation_source = 'import_manus'`*.

| ID | Gate | Verification command |
|----|------|----------------------|
| `A5.1` | **HEADLINE** Fixture import emits 7 audit events with `reservation_source = 'import_manus'` | `cargo test -p spendguard-importer-manus --test fixture_e2e fixture_import_emits_seven_audit_events fixture_import_each_row_tagged_import_manus` exits 0 |
| `A5.2` | Every emitted row tagged `import_source = 'manus_admin_usage'` | `cargo test -p spendguard-importer-manus --test fixture_e2e fixture_import_each_row_tagged_manus_admin_usage` exits 0 |
| `A5.3` | No `ledger_entries` writes | `cargo test -p spendguard-importer-manus --test fixture_e2e fixture_import_no_ledger_write` exits 0 |
| `A5.4` | Team-plan tier amount math correct (1010 credits × 20_526 = 20_731_260 micro-USD) | `cargo test -p spendguard-importer-manus --test fixture_e2e fixture_import_team_plan_amounts_match_pricing` exits 0 |
| `A5.5` | BYOK tier amount is zero | `cargo test -p spendguard-importer-manus --test fixture_e2e fixture_import_enterprise_byok_amount_is_zero` exits 0 |
| `A5.6` | Fixture import is idempotent across two runs | `cargo test -p spendguard-importer-manus --test fixture_e2e fixture_import_idempotent_across_two_runs` exits 0 |
| `A5.7` | CloudEvent type emitted is `spendguard.audit.import.manus_credit` | `cargo test -p spendguard-importer-manus --test fixture_e2e fixture_import_cloudevent_type_is_manus_credit` exits 0 |

## 6. Live-mode mock gate (opt-in)

These run only with `--features live`; they prove the HTTP client works against a mock without requiring a real Manus account.

| ID | Gate | Verification command |
|----|------|----------------------|
| `A6.1` | All `live_mock` tests green with feature on | `cargo test -p spendguard-importer-manus --features live --test live_mock` exits 0 |
| `A6.2` | Bearer token header sent to mock | covered by `live_poll_usage_sends_bearer_token` inside `A6.1` |
| `A6.3` | Cursor pagination drains all pages | covered by `live_poll_usage_handles_cursor_pagination` inside `A6.1` |
| `A6.4` | Malformed records skipped with WARN, not panicked | covered by `live_poll_usage_skips_malformed_records_with_warn` inside `A6.1` |
| `A6.5` | 401 error message redacts token | covered by `live_poll_usage_http_401_returns_err_with_redacted_token` inside `A6.1` |

`A6.x` is NOT a default-build gate. The merge gate is `A5.x` (fixture-driven). Per the deliverable prompt: *Live mode gated behind `MANUS_API_TOKEN`*.

## 7. Schema migration gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A7.1` | Migrations 0047 + 0048 apply cleanly to a fresh PG 16 + prior migrations | `make -C deploy/demo demo-up && psql "$DATABASE_URL" -c "SELECT 1 FROM pg_constraint WHERE conname = 'audit_outbox_reservation_source_check';" \| grep -q '1 row'` |
| `A7.2` | Migration idempotency: apply twice → no error | `psql "$DATABASE_URL" -f services/canonical_ingest/migrations/0047_audit_outbox_extend_reservation_source.sql` succeeds twice |
| `A7.3` | CHECK accepts `import_manus`, rejects `wat` | `psql "$DATABASE_URL" -c "INSERT INTO audit_outbox (…) VALUES (…, 'import_manus', …);"` succeeds; replacing with `'wat'` returns SQLSTATE `23514` |
| `A7.4` | Partial index `idx_audit_outbox_import_manus` exists with correct predicate | `psql "$DATABASE_URL" -c "SELECT indexdef FROM pg_indexes WHERE indexname = 'idx_audit_outbox_import_manus';" \| grep -qE "reservation_source = 'import_manus'"` |
| `A7.5` | CHECK accepts NULL `import_source` (live proxy path unchanged) | `psql "$DATABASE_URL" -c "INSERT INTO audit_outbox (…, import_source) VALUES (…, NULL);"` succeeds |
| `A7.6` | Migration 0047/0048 listed in `migration_inventory.toml` with checksum | `grep -qE '0047_audit_outbox_extend_reservation_source' services/canonical_ingest/migration_inventory.toml && grep -qE '0048_audit_outbox_extend_import_source' services/canonical_ingest/migration_inventory.toml` |
| `A7.7` | Down-migrations restore D13 narrower CHECKs cleanly | `psql "$DATABASE_URL" -f services/canonical_ingest/migrations/down/0048_audit_outbox_extend_import_source_down.sql && psql "$DATABASE_URL" -f services/canonical_ingest/migrations/down/0047_audit_outbox_extend_reservation_source_down.sql` succeed; replacing `import_manus` then fails CHECK |

## 8. Demo-mode regression gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A8.1` | **HEADLINE** `make -C deploy/demo demo-import-manus-fixture` exits 0 | Replay `manus_usage.json` → asserts 7 import rows + no ledger rows + correct team_plan total. |
| `A8.2` | Demo idempotent: re-running the target does not double rows | `make -C deploy/demo demo-import-manus-fixture && make -C deploy/demo demo-import-manus-fixture` both exit 0 and post-run row count is 7. |
| `A8.3` | Pre-existing D13 subscription_meter demo still green | `make -C deploy/demo demo-verify-subscription-meter-claude-code` exits 0 |
| `A8.4` | Pre-existing BYOK litellm demo still green | `make -C deploy/demo demo-verify-litellm-real` exits 0 |
| `A8.5` | Pre-existing pricing demo still green | `make -C deploy/demo demo-verify-pricing` exits 0 |
| `A8.6` | `make demo-clean` purges Manus importer rows | After clean, `SELECT count(*) FROM audit_outbox WHERE reservation_source = 'import_manus'` returns 0. |

## 9. Performance gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A9.1` | `load_fixture` 8-session p99 < 10ms | `cargo test -p spendguard-importer-manus --release -- --ignored load_fixture_8_sessions_under_10ms` exits 0 |
| `A9.2` | `credit_to_usd_micros` p99 < 100ns | `cargo test -p spendguard-importer-manus --release -- --ignored credit_to_usd_micros_under_100ns` exits 0 |
| `A9.3` | `import_record_to_audit_row` p99 < 500ns | `cargo test -p spendguard-importer-manus --release -- --ignored import_record_to_audit_row_under_500ns` exits 0 |

## 10. Security gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A10.1` | No fixture contains a real Manus token sentinel-only check | `grep -rE '(sk-[A-Za-z0-9_-]{40,}\|eyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,})' services/importer_manus/tests/fixtures/ \| grep -v FAKE_` exits 1 (no matches) |
| `A10.2` | `live` mode does not log full token (200-iteration tracing capture) | `cargo test -p spendguard-importer-manus --features live live_mode_does_not_log_full_token live_mode_does_not_log_full_token_on_error` exits 0 |
| `A10.3` | Fixture `PROVENANCE.md` pins redaction script SHA-256 | `grep -qE 'sha256:[0-9a-f]{64}' services/importer_manus/tests/fixtures/PROVENANCE.md` |
| `A10.4` | Workspace IDs in fixture are sentinel-prefixed | `jq -r '.sessions[].workspace_id' services/importer_manus/tests/fixtures/manus_usage.json \| grep -vE '^ws_FAKE_'` exits 1 (no matches) |
| `A10.5` | Session IDs in fixture are sentinel-prefixed | `jq -r '.sessions[].session_id' services/importer_manus/tests/fixtures/manus_usage.json \| grep -vE '^ses_FAKE_'` exits 1 (no matches) |
| `A10.6` | SQL injection — workspace IDs with quotes round-trip safely | `cargo test -p spendguard-importer-manus fixture_with_quotes_in_workspace_id_safe` exits 0 |
| `A10.7` | `live` HTTP client uses `rustls-tls` not `native-tls` (consistent with project policy) | `grep -qE 'rustls-tls' services/importer_manus/Cargo.toml && ! grep -qE 'native-tls' services/importer_manus/Cargo.toml` |

## 11. Documentation gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A11.1` | Integration doc explicitly says "post-hoc reconciliation, not enforcement" | `grep -qE 'reconciliation\|not.*enforce\|cannot.*enforce' docs/site-v2/src/content/docs/integrations/manus-importer.md` |
| `A11.2` | Doc explains both fixture mode and live mode (`MANUS_API_TOKEN`) | `grep -qE 'MANUS_API_TOKEN' docs/site-v2/src/content/docs/integrations/manus-importer.md` |
| `A11.3` | Doc explains the three pricing tiers | `grep -qE 'team_plan\|enterprise\|enterprise_byok' docs/site-v2/src/content/docs/integrations/manus-importer.md` (all three matched) |
| `A11.4` | Doc cross-links to D14 Devin importer + D16 Genspark importer (or notes them as sibling work) | `grep -qE 'Devin\|Genspark\|D14\|D16' docs/site-v2/src/content/docs/integrations/manus-importer.md` |
| `A11.5` | Doc cross-links to strategy memo Archetype IV section | `grep -qE 'Archetype IV\|framework-coverage' docs/site-v2/src/content/docs/integrations/manus-importer.md` |
| `A11.6` | Embedded JSON examples wrapped in `is:raw` (Starlight Astro convention) | `grep -qE 'is:raw' docs/site-v2/src/content/docs/integrations/manus-importer.md` |
| `A11.7` | Crate README explains stub vs live mode | `grep -qE 'fixture-driven\|live.*MANUS_API_TOKEN' services/importer_manus/README.md` |

## 12. Anti-regression gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A12.1` | Existing BYOK Anthropic integration test still green | `cargo test -p spendguard-egress-proxy routes_anthropic_messages` exits 0 |
| `A12.2` | Existing sidecar ledger-write integration test still green | `cargo test -p spendguard-sidecar reserve_v2_commit_estimated_writes_ledger_entries` exits 0 |
| `A12.3` | D13 subscription meter integration test still green | `cargo test -p spendguard-egress-proxy --test subscription_meter_e2e claude_code_pro_session_meters_correctly` exits 0 |
| `A12.4` | `pre-D15` `audit_outbox.reservation_source` values still valid post-migration | `psql "$DATABASE_URL" -c "INSERT INTO audit_outbox (…, reservation_source) VALUES (…, 'byok');"` succeeds (regression: D13 baseline) |
| `A12.5` | No new dependency in workspace `Cargo.lock` outside the importer crate | `git diff Cargo.lock` shows additions only under the importer's transitive cone — no unrelated upgrades. |

`A12.x` collectively ensures D15 is purely additive — no D13 / BYOK regression.
