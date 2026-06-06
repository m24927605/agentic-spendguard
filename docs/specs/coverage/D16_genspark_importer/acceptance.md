# D16 — Acceptance Gates

Per build plan §3 + §3 feasibility rule for architecturally-unreachable deliverables: every gate listed here must be **100% feasible** at slice-spec time — runnable in the current repo state, no third-party action (no live Genspark API call) required, reproducible by the `superpowers:code-reviewer` skill. Per build plan §3 line "feasibility = synthetic audit event emitted is the primary gate, not live import succeeded", the headline gate is the fixture-mode import (§9), not a live admin-API call.

## 1. Repository-state gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A1.1` | Crate exists at `services/importer_genspark/Cargo.toml`, package name `spendguard-importer-genspark` | `test -f services/importer_genspark/Cargo.toml && grep -qE 'name = "spendguard-importer-genspark"' services/importer_genspark/Cargo.toml` |
| `A1.2` | `services/importer_genspark/src/lib.rs` exports `import_window_from_fixture` | `grep -qE 'pub async fn import_window_from_fixture' services/importer_genspark/src/lib.rs` |
| `A1.3` | `services/importer_genspark/src/audit.rs` exports `import_record_to_audit_row` (pure) | `grep -qE 'pub fn import_record_to_audit_row' services/importer_genspark/src/audit.rs` |
| `A1.4` | `services/importer_genspark/src/price.rs` exports `GensparkPriceTable::load` + `credits_to_micro_usd` | `grep -qE 'pub fn load' services/importer_genspark/src/price.rs && grep -qE 'pub fn credits_to_micro_usd' services/importer_genspark/src/price.rs` |
| `A1.5` | `services/importer_genspark/src/live.rs` is gated `#[cfg(feature = "live")]` | `grep -qE '#\[cfg\(feature = "live"\)\]' services/importer_genspark/src/live.rs` |
| `A1.6` | `services/importer_genspark/config/genspark_credit_price.toml` exists with `pricing_version = "genspark-2026-06-06"` | `grep -qE 'pricing_version = "genspark-2026-06-06"' services/importer_genspark/config/genspark_credit_price.toml` |
| `A1.7` | Migration `0053_audit_outbox_import_genspark.sql` exists | `test -f services/canonical_ingest/migrations/0053_audit_outbox_import_genspark.sql` |
| `A1.8` | Down migration `down/0053_audit_outbox_import_genspark.sql` exists | `test -f services/canonical_ingest/migrations/down/0053_audit_outbox_import_genspark.sql` |
| `A1.9` | Three fixtures committed | `for f in genspark_usage genspark_usage_premium genspark_usage_unknown_plan; do test -f "services/importer_genspark/tests/fixtures/$f.json" || exit 1; done` |
| `A1.10` | `services/importer_genspark/tests/fixtures/PROVENANCE.md` exists with no-PII assertion | `grep -qE 'No PII|no PII' services/importer_genspark/tests/fixtures/PROVENANCE.md` |
| `A1.11` | Workspace `Cargo.toml` excludes the new crate (matches project convention) | `grep -qE '"services/importer_genspark"' Cargo.toml` |
| `A1.12` | Demo verifier SQL committed | `test -f deploy/demo/verify_step_import_genspark_fixture.sql` |
| `A1.13` | Demo runtime script committed | `test -f deploy/demo/runtime/import_genspark_demo.sh && test -x deploy/demo/runtime/import_genspark_demo.sh` |
| `A1.14` | Starlight integration doc page exists | `test -f docs/site-v2/src/content/docs/integrations/genspark-billing-importer.md` |
| `A1.15` | README adapter table includes a "Genspark billing importer" row | `grep -q 'Genspark billing importer' README.md` |

## 2. Build gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A2.1` | Crate builds with default features (no `live`) | `cargo build --manifest-path services/importer_genspark/Cargo.toml --locked` exits 0 |
| `A2.2` | Crate builds with `--features live` | `cargo build --manifest-path services/importer_genspark/Cargo.toml --features live --locked` exits 0 |
| `A2.3` | Crate builds in release mode | `cargo build --manifest-path services/importer_genspark/Cargo.toml --release --locked` exits 0 |
| `A2.4` | Default-features build pulls NO `reqwest` | `cargo tree --manifest-path services/importer_genspark/Cargo.toml -e=normal | grep -q reqwest` exits non-zero (no match) |
| `A2.5` | Default-features build pulls NO `secrecy` | `cargo tree --manifest-path services/importer_genspark/Cargo.toml -e=normal | grep -q '^secrecy'` exits non-zero |
| `A2.6` | `live`-features build pulls `reqwest` with rustls-tls (no native-tls) | `cargo tree --manifest-path services/importer_genspark/Cargo.toml --features live -e=normal | grep -E 'reqwest.*(rustls-tls)' \|\| cargo tree --manifest-path services/importer_genspark/Cargo.toml --features live -e=normal | grep -q rustls` |
| `A2.7` | No new warnings (default features) | `cargo build --manifest-path services/importer_genspark/Cargo.toml -- -D warnings` exits 0 |
| `A2.8` | Clippy clean (default features) | `cargo clippy --manifest-path services/importer_genspark/Cargo.toml --all-targets -- -D warnings` exits 0 |
| `A2.9` | Clippy clean (`--features live`) | `cargo clippy --manifest-path services/importer_genspark/Cargo.toml --all-targets --features live -- -D warnings` exits 0 |
| `A2.10` | `cargo deny check` passes | `cargo deny check --manifest-path services/importer_genspark/Cargo.toml` exits 0 (or workspace `cargo deny check` if the crate joins the workspace dep manifest) |

## 3. Unit-test gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A3.1` | Price-table unit tests green | `cargo test --manifest-path services/importer_genspark/Cargo.toml price::tests` exits 0 |
| `A3.2` | `import_record_to_audit_row` purity + correctness tests green | `cargo test --manifest-path services/importer_genspark/Cargo.toml audit::tests` exits 0 |
| `A3.3` | Admin-API parser tests green | `cargo test --manifest-path services/importer_genspark/Cargo.toml record::tests` exits 0 |
| `A3.4` | Live-mode gating tests green (with feature) | `cargo test --manifest-path services/importer_genspark/Cargo.toml --features live live::tests` exits 0 |
| `A3.5` | Token-never-logged enforcement test green | `cargo test --manifest-path services/importer_genspark/Cargo.toml --features live live_client_token_never_logged` exits 0 |
| `A3.6` | Unknown-plan fallback test green | `cargo test --manifest-path services/importer_genspark/Cargo.toml record_to_row_unknown_plan_amount_zero record_to_row_unknown_plan_reason_code` exits 0 |
| `A3.7` | Pricing version propagation test green | `cargo test --manifest-path services/importer_genspark/Cargo.toml record_to_row_pricing_version_propagates` exits 0 |

## 4. Fixture-driven integration-test gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A4.1` | All fixture-import integration tests green | `cargo test --manifest-path services/importer_genspark/Cargo.toml --test fixture_import` exits 0 |
| `A4.2` | Plus-tier fixture writes exactly the expected number of rows | `cargo test --manifest-path services/importer_genspark/Cargo.toml --test fixture_import fixture_plus_tier_window_writes_two_rows` exits 0 |
| `A4.3` | Importer does NOT write to `ledger_entries` (anti-regression vs. D13 §4.3 invariant) | `cargo test --manifest-path services/importer_genspark/Cargo.toml --test fixture_import fixture_plus_tier_does_not_write_ledger fixture_plus_tier_does_not_write_reservations` exits 0 |
| `A4.4` | Unknown-plan fallback writes row with `amount_micro_usd = 0` + `reason_code = 'genspark_plan_unknown'` | `cargo test --manifest-path services/importer_genspark/Cargo.toml --test fixture_import fixture_unknown_plan_writes_unpriced_row` exits 0 |
| `A4.5` | CloudEvent type test green | `cargo test --manifest-path services/importer_genspark/Cargo.toml --test fixture_import fixture_cloudevent_type_correct` exits 0 |
| `A4.6` | Fixture provenance gates green | `cargo test --manifest-path services/importer_genspark/Cargo.toml fixture_has_provenance_md fixture_no_real_workspace_ids fixture_no_prompt_content` exits 0 |

## 5. Schema migration gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A5.1` | Migration 0053 applies cleanly to a fresh PG 16 instance with D13's 0044+0046 already applied | `make -C deploy/demo demo-up && psql "$DATABASE_URL" -f services/canonical_ingest/migrations/0053_audit_outbox_import_genspark.sql` succeeds |
| `A5.2` | Migration idempotency: apply twice → no error | `psql "$DATABASE_URL" -f services/canonical_ingest/migrations/0053_audit_outbox_import_genspark.sql` succeeds twice in a row |
| `A5.3` | CHECK constraint accepts `'import_genspark'` reservation_source | `psql "$DATABASE_URL" -c "INSERT INTO audit_outbox (…, reservation_source) VALUES (…, 'import_genspark');"` succeeds |
| `A5.4` | CHECK constraint accepts `'genspark_billing'` import_source | `psql "$DATABASE_URL" -c "INSERT INTO audit_outbox (…, import_source) VALUES (…, 'genspark_billing');"` succeeds |
| `A5.5` | CHECK constraint rejects unknown reservation_source | `psql "$DATABASE_URL" -c "INSERT INTO audit_outbox (…, reservation_source) VALUES (…, 'wat');"` returns SQLSTATE `23514` |
| `A5.6` | D13's existing values still accepted (anti-regression) | `psql "$DATABASE_URL" -c "INSERT INTO audit_outbox (…) VALUES (…, 'byok');"` and `(…, 'subscription_meter')` both succeed |
| `A5.7` | Partial index `idx_audit_outbox_import_genspark` exists with correct predicate | `psql "$DATABASE_URL" -c "SELECT indexdef FROM pg_indexes WHERE indexname = 'idx_audit_outbox_import_genspark';" | grep -qE "reservation_source = 'import_genspark'"` |
| `A5.8` | Migration listed in `migration_inventory.toml` | `grep -qE '0053_audit_outbox_import_genspark' services/canonical_ingest/migration_inventory.toml` |
| `A5.9` | Rollback works on a fresh DB (no import_genspark rows) | `psql "$DATABASE_URL" -f services/canonical_ingest/migrations/down/0053_audit_outbox_import_genspark.sql` succeeds |
| `A5.10` | Rollback fails clearly when import_genspark rows exist (operator-obligation regression) | Insert one row → rollback returns SQLSTATE `23514` |

## 6. Demo-mode regression gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A6.1` | `make -C deploy/demo demo-verify-import-genspark-fixture` exits 0 | Replays the Plus-tier fixture; verifier SQL asserts ≥ 2 audit rows + sum > 0 + zero ledger rows. |
| `A6.2` | Verifier SQL is committed and self-contained (no external state) | `test -f deploy/demo/verify_step_import_genspark_fixture.sql && head -3 deploy/demo/verify_step_import_genspark_fixture.sql` confirms `DO $$` block. |
| `A6.3` | Pre-existing BYOK demo regression: `make -C deploy/demo demo-verify-litellm-real` still exits 0 | D16 must not break the BYOK ledger path. |
| `A6.4` | Pre-existing pricing demo regression: `make -C deploy/demo demo-verify-pricing` still exits 0 | Other pricing flows unaffected. |
| `A6.5` | `make -C deploy/demo demo-clean` removes D16 rows | After clean, `SELECT count(*) FROM audit_outbox WHERE reservation_source = 'import_genspark'` returns 0. |
| `A6.6` | Demo runs even when `live` feature is OFF (default build) | The demo binary built without `--features live` succeeds in fixture mode. |

## 7. Live-mode gating gates

D16 ships the `live` feature as a typed stub. Per design §3.3 + §6 decision #4, the live mode has a compile-time gate (`live` Cargo feature) AND a runtime gate (`GENSPARK_API_TOKEN` ≥ 32 chars). Gates verify both layers without ever making a real outbound HTTP call.

| ID | Gate | Verification command |
|----|------|----------------------|
| `A7.1` | Without `live` feature, `--workspace` arg returns a clear error | `cargo run --manifest-path services/importer_genspark/Cargo.toml --bin spendguard-importer-genspark -- --workspace foo --window-from 2026-06-01T00:00:00Z --window-to 2026-06-02T00:00:00Z --database-url $DATABASE_URL` exits non-zero with stderr containing "live" |
| `A7.2` | With `live` feature but no `GENSPARK_API_TOKEN`, client construction fails clearly | `cargo test --manifest-path services/importer_genspark/Cargo.toml --features live live_client_from_env_missing_token_returns_err` exits 0 |
| `A7.3` | Empty token → reject | `GENSPARK_API_TOKEN="" cargo test … --features live live_client_from_env_empty_token_returns_err` exits 0 |
| `A7.4` | Token < 32 chars → reject | `GENSPARK_API_TOKEN="TODO" cargo test … --features live live_client_from_env_short_token_returns_err` exits 0 |
| `A7.5` | Token stored in `SecretString` (never logged) | `cargo test … --features live live_client_token_never_logged` exits 0 |

## 8. Security gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A8.1` | Fixtures contain no real Genspark workspace IDs | `cargo test --manifest-path services/importer_genspark/Cargo.toml fixture_no_real_workspace_ids` exits 0 |
| `A8.2` | Fixtures contain no prompt content | `cargo test --manifest-path services/importer_genspark/Cargo.toml fixture_no_prompt_content` exits 0 |
| `A8.3` | `GENSPARK_API_TOKEN` is never logged | `cargo test … --features live live_client_token_never_logged` exits 0 |
| `A8.4` | Live client uses rustls-tls (no native-tls) | `cargo tree --manifest-path services/importer_genspark/Cargo.toml --features live -e=normal | grep -q rustls && cargo tree --manifest-path services/importer_genspark/Cargo.toml --features live -e=normal | grep -q native-tls` exits non-zero on second grep |
| `A8.5` | Importer does NOT write to `ledger_entries` (cost-basis double-count prevention) | `A4.3` |
| `A8.6` | Unknown plan is surfaced as unpriced (`reason_code = "genspark_plan_unknown"`), not silently zero | `A4.4` |
| `A8.7` | Migration 0053 does NOT narrow D13's CHECK set (regression check) | `A5.6` |

## 9. Acceptance scenario gate (primary headline gate)

**The headline acceptance scenario** (from the deliverable prompt):

> Fixture-driven test imports recorded `genspark_usage.json` and emits synthetic audit events with `reservation_source = 'import_genspark'`. Live mode gated behind `GENSPARK_API_TOKEN`.

This is verified by `A9.1` + `A9.2` running end-to-end against the demo stack:

| ID | Gate | Verification command |
|----|------|----------------------|
| `A9.1` | Fixture-mode import emits audit events with `reservation_source = 'import_genspark'` | `make -C deploy/demo demo-verify-import-genspark-fixture` exits 0; verifier SQL asserts `count >= 2 WHERE reservation_source = 'import_genspark' AND import_source = 'genspark_billing'`, `sum(amount_micro_usd) > 0`, AND `0 rows in ledger_entries`. |
| `A9.2` | Live mode is gated behind `GENSPARK_API_TOKEN` AND `live` Cargo feature | `A7.1` (no `live` feature → live mode unreachable) AND `A7.2` (no token → live client construction fails) BOTH exit 0. |

`A9.1` is the **merge-blocking gate** that maps directly to the deliverable prompt's primary acceptance criterion. `A9.2` is the secondary merge-blocking gate covering the "live mode gated behind `GENSPARK_API_TOKEN`" half of the prompt.

## 10. Documentation gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A10.1` | Starlight doc page exists | `test -f docs/site-v2/src/content/docs/integrations/genspark-billing-importer.md` |
| `A10.2` | Doc page explicitly says "reconciliation, not enforcement" | `grep -qE 'reconciliation.*not.*enforce|cannot enforce|post-hoc' docs/site-v2/src/content/docs/integrations/genspark-billing-importer.md` |
| `A10.3` | Doc page cross-links to strategy memo §"Archetype IV" | `grep -qE 'Archetype IV|framework-coverage' docs/site-v2/src/content/docs/integrations/genspark-billing-importer.md` |
| `A10.4` | Doc page mentions `GENSPARK_API_TOKEN` env var | `grep -q 'GENSPARK_API_TOKEN' docs/site-v2/src/content/docs/integrations/genspark-billing-importer.md` |
| `A10.5` | Doc page mentions the unknown-plan fallback semantic | `grep -qE 'unknown.*plan|genspark_plan_unknown|unpriced' docs/site-v2/src/content/docs/integrations/genspark-billing-importer.md` |
| `A10.6` | Doc page mentions the `spendguard.audit.import.genspark_credit` CloudEvent type | `grep -q 'spendguard.audit.import.genspark_credit' docs/site-v2/src/content/docs/integrations/genspark-billing-importer.md` |
| `A10.7` | Crate README exists and explicitly tags the crate as an importer | `test -f services/importer_genspark/README.md && grep -qE 'Genspark|importer|reconciliation' services/importer_genspark/README.md` |
| `A10.8` | Embedded code/JSON examples in Starlight doc are wrapped in `is:raw` (project convention) | `grep -q 'is:raw' docs/site-v2/src/content/docs/integrations/genspark-billing-importer.md` (asserted whenever a JSON/code block is embedded) |

## 11. Anti-regression gate

| ID | Gate | Verification command |
|----|------|----------------------|
| `A11.1` | D13 `audit_outbox.reservation_source` column behaviour unchanged for `'byok'` and `'subscription_meter'` rows | `A5.6` |
| `A11.2` | D13 `audit_outbox.import_source` column behaviour unchanged for `'anthropic_console_usage'` and `'openai_admin_usage'` | `psql "$DATABASE_URL" -c "INSERT INTO audit_outbox (…, import_source) VALUES (…, 'anthropic_console_usage'), (…, 'openai_admin_usage');"` succeeds |
| `A11.3` | Existing BYOK ledger path untouched | `cargo test -p spendguard-sidecar reserve_v2_commit_estimated_writes_ledger_entries` exits 0 |
| `A11.4` | Existing demo `demo-verify-litellm-real` green | `A6.3` |
| `A11.5` | No proto-level changes (D16 reuses D13's `reservation_source` + `import_source` columns; the new values land in the CHECK constraint only) | `git diff main -- proto/` shows no D16-attributable proto changes |

`A11.x` collectively ensures D16 is purely additive — no D13 / BYOK regression.

## 12. Slice → gate mapping

| Slice | Gates covered |
|-------|---------------|
| `COV_84_d16_genspark_crate_scaffold` | A1.1, A1.11, A2.1, A2.3 |
| `COV_85_d16_credit_price_and_record_to_row` | A1.3, A1.4, A1.6, A3.1, A3.2, A3.6, A3.7, A8.6 |
| `COV_86_d16_fixture_import_path` | A1.2, A1.7-A1.10, A1.12-A1.13, A4.1-A4.6, A5.1-A5.10, A6.1-A6.2, A8.1-A8.2, A8.5, A8.7, A9.1, A11.1-A11.2 |
| `COV_87_d16_live_http_client` | A1.5, A2.2, A2.4-A2.6, A3.4-A3.5, A7.1-A7.5, A8.3-A8.4, A9.2 |
| `COV_88_d16_demo_and_docs` | A1.14-A1.15, A6.3-A6.6, A10.1-A10.8, A11.3-A11.5 |
