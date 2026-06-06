# D16 — Tests

Companion to [`design.md`](design.md) and [`implementation.md`](implementation.md). Defines unit, fixture-driven integration, demo regression, migration, and gating coverage. None require live `GENSPARK_API_TOKEN` or live admin-API access — all gates run in `cargo test` + `make -C deploy/demo demo-verify-*`.

## 1. Unit tests

### 1.1 `price.rs` — credit → USD conversion

| Test | Asserts |
|------|---------|
| `price_table_load_parses_committed_toml` | Loading `config/genspark_credit_price.toml` succeeds; `pricing_version = "genspark-2026-06-06"`. |
| `price_table_load_fills_effective_per_credit` | After `load`, `plans["plus"].effective_usd_per_credit ≈ 0.001999`. |
| `credits_to_micro_usd_plus_tier_known_value` | `credits_to_micro_usd("plus", 1000) == Some(1999)` (1000 × 0.001999 USD × 1e6 = 1999 micro-USD). |
| `credits_to_micro_usd_pro_tier_known_value` | Same shape for "pro" plan. |
| `credits_to_micro_usd_premium_tier_known_value` | Same shape for "premium" plan. |
| `credits_to_micro_usd_unknown_plan_returns_none` | `credits_to_micro_usd("enterprise", 500) == None`. |
| `credits_to_micro_usd_unknown_returns_none_not_zero` | Distinguishes "unknown plan" from "zero credits" — `None` vs `Some(0)`. |
| `credits_to_micro_usd_zero_credits_returns_zero` | `credits_to_micro_usd("plus", 0) == Some(0)`. |
| `credits_to_micro_usd_negative_credits_returns_negative` | Genspark records never return negative, but the type allows it — saturating arithmetic, no panic. |
| `price_table_override_via_env_path` | `GENSPARK_PRICE_TABLE_PATH` set → loader reads that path instead of the default. |
| `price_table_malformed_toml_returns_err` | Garbage TOML → `Err(ImporterError::PriceTable)`, no panic. |
| `price_table_missing_pricing_version_returns_err` | TOML missing `pricing_version` → `Err`. |
| `price_table_zero_monthly_credits_returns_none` | `monthly_credits = 0` for some plan → division avoided, `credits_to_micro_usd` returns `None`. |

### 1.2 `audit.rs` — `import_record_to_audit_row` (pure)

| Test | Asserts |
|------|---------|
| `record_to_row_sets_reservation_source` | Output row has `reservation_source == "import_genspark"`. |
| `record_to_row_sets_import_source` | `import_source == Some("genspark_billing")`. |
| `record_to_row_sets_tenant_id_from_workspace_id` | `workspace_id = "FAKE_ws_alpha"` → `row.tenant_id == "FAKE_ws_alpha"`. |
| `record_to_row_model_is_synthetic_genspark_prefix` | `plan = "plus"` → `row.model == "genspark/plus"`. |
| `record_to_row_input_output_tokens_zero` | Both token counts zero (admin API does not return token counts). |
| `record_to_row_amount_priced_correctly_plus` | `credits_consumed = 1000`, plan `plus` → `amount_micro_usd == 1999`. |
| `record_to_row_pricing_version_propagates` | Row's `pricing_version == price.pricing_version`. |
| `record_to_row_occurred_at_uses_window_end` | `row.occurred_at == rec.window_end` (audit semantic: "as of end of window"). |
| `record_to_row_unknown_plan_amount_zero` | `plan = "enterprise"` (not in table) → `amount_micro_usd == 0`. |
| `record_to_row_unknown_plan_reason_code` | `plan = "enterprise"` → `reason_code == Some("genspark_plan_unknown")`. |
| `record_to_row_known_plan_no_reason_code` | `plan = "plus"` → `reason_code == None`. |
| `record_to_row_is_pure` | Same input twice → byte-identical `AuditRow` output. |
| `record_to_row_no_global_state` | Verified by signature `fn import_record_to_audit_row(rec: &…, price: &…) -> AuditRow` — all args borrowed, no `&mut`. |

### 1.3 `record.rs` — admin-API parser

| Test | Asserts |
|------|---------|
| `parse_admin_response_basic_shape` | Parses `genspark_usage.json` fixture → `resp.records.len() >= 2`. |
| `parse_admin_response_window_bounds` | `resp.window_start < resp.window_end`, both RFC3339-parseable. |
| `parse_admin_response_pagination_nullable` | `pagination: null` in JSON → `resp.pagination == None`. |
| `parse_admin_response_pagination_present` | When fixture has cursor → `resp.pagination.next_cursor == Some("…")`. |
| `parse_admin_response_unknown_fields_ignored` | Adding `"random_new_field": 42` to the JSON → still parses (serde default is forgiving; protects against vendor schema additions). |
| `parse_admin_response_missing_credits_consumed_errs` | Record missing `credits_consumed` → `Err`. |
| `parse_admin_response_task_category_optional` | Records without `task_category` parse cleanly. |

### 1.4 `live.rs` — gating (compiles only with `--features live`)

These tests live behind `#[cfg(feature = "live")]` and run via `cargo test --features live`. They never call the real admin API — they exercise the env-var gate only.

| Test | Asserts |
|------|---------|
| `live_client_from_env_missing_token_returns_err` | `GENSPARK_API_TOKEN` unset → `from_env()` returns `Err`. |
| `live_client_from_env_empty_token_returns_err` | `GENSPARK_API_TOKEN = ""` → `Err`. |
| `live_client_from_env_short_token_returns_err` | `GENSPARK_API_TOKEN = "TODO"` (< 32 chars) → `Err` with message containing min length. |
| `live_client_from_env_valid_token_returns_ok` | 64-char token → `Ok(GensparkAdminClient)`. |
| `live_client_token_never_logged` | After construction, `tracing-test` subscriber captures contain no substring of the token. |
| `live_client_token_stored_in_secrecy` | Verified by signature: `token: SecretString` — `Debug` impl redacts. |

## 2. Fixture-driven integration tests

`services/importer_genspark/tests/fixture_import.rs` — spins up an ephemeral PG (via `sqlx` test fixtures + a tempdir DSN), applies migrations 0044 + 0046 + 0053, runs `import_window_from_fixture`, and asserts row state.

| Test | Fixture | Asserts |
|------|---------|---------|
| `fixture_plus_tier_window_writes_two_rows` | `genspark_usage.json` | After `import_window_from_fixture`, `SELECT count(*) FROM audit_outbox WHERE reservation_source = 'import_genspark'` returns exactly 2. |
| `fixture_plus_tier_window_writes_correct_tenant_id` | same | Both rows have `tenant_id = 'FAKE_ws_alpha'`. |
| `fixture_plus_tier_window_writes_priced_aggregate` | same | `SUM(amount_micro_usd)` matches hand-computed expected value from credits × $/credit. |
| `fixture_plus_tier_does_not_write_ledger` | same | `SELECT count(*) FROM ledger_entries WHERE tenant_id LIKE 'FAKE_ws_%'` returns 0. |
| `fixture_plus_tier_does_not_write_reservations` | same | `SELECT count(*) FROM reservations WHERE tenant_id LIKE 'FAKE_ws_%'` returns 0. |
| `fixture_premium_tier_multi_workspace` | `genspark_usage_premium.json` | Distinct `tenant_id` values across rows; `SUM` matches Premium pricing. |
| `fixture_unknown_plan_writes_unpriced_row` | `genspark_usage_unknown_plan.json` | Row written with `amount_micro_usd = 0` AND `reason_code = 'genspark_plan_unknown'`. |
| `fixture_unknown_plan_still_visible_on_dashboard` | same | Row matches the partial index `idx_audit_outbox_import_genspark` predicate (i.e. `reservation_source = 'import_genspark'`). |
| `fixture_idempotent_replay` | `genspark_usage.json` | Running the importer twice on the same fixture writes 2 + 2 = 4 rows (idempotency is the operator's responsibility via window dedup, not the importer's — this asserts the contract). |
| `fixture_pricing_version_propagates_to_row` | `genspark_usage.json` | Every row's `pricing_version` column equals `"genspark-2026-06-06"`. |
| `fixture_cloudevent_type_correct` | `genspark_usage.json` | Built CloudEvent has `type = "spendguard.audit.import.genspark_credit"`. |
| `fixture_cloudevent_subject_is_tenant_id` | same | CloudEvent `subject == row.tenant_id`. |
| `fixture_window_end_used_as_occurred_at` | same | `row.occurred_at == rec.window_end` for every record. |

### 2.1 Fixture provenance gate

| Test | Asserts |
|------|---------|
| `fixture_has_provenance_md` | `tests/fixtures/PROVENANCE.md` exists and is non-empty. |
| `fixture_no_real_workspace_ids` | grep across every `.json` fixture: no string matching `ws_[A-Za-z0-9]{16,}` that isn't `FAKE_ws_*`. |
| `fixture_no_prompt_content` | grep across every `.json` fixture: no `"content":` field present (admin API doesn't return prompts — protects against accidental capture pollution). |

## 3. Schema migration tests

`services/importer_genspark/tests/migration.rs`:

| Test | Asserts |
|------|---------|
| `0053_apply_succeeds_on_fresh_db` | Apply 0044 → 0046 → 0053 in order; no error. |
| `0053_apply_is_idempotent` | Apply 0053 twice → no error (CHECK drop-then-add with `IF EXISTS`). |
| `0053_rollback_succeeds_when_no_genspark_rows` | Run `down/0053…` on empty table → succeeds; CHECK reverts to D13 set. |
| `0053_rollback_fails_when_genspark_rows_present` | Insert one `reservation_source = 'import_genspark'` row, then run rollback → fails with SQLSTATE `23514` (CHECK violation on existing data). Documents the operator obligation. |
| `0053_check_rejects_unknown_reservation_source` | `INSERT … reservation_source = 'wat'` → SQLSTATE `23514`. |
| `0053_check_accepts_import_genspark` | `INSERT … reservation_source = 'import_genspark'` → succeeds. |
| `0053_check_accepts_byok_and_subscription_meter_unchanged` | Both D13 values still accepted (regression: 0053 must not narrow). |
| `0053_import_source_check_accepts_genspark_billing` | `INSERT … import_source = 'genspark_billing'` → succeeds. |
| `0053_import_source_check_accepts_anthropic_unchanged` | `INSERT … import_source = 'anthropic_console_usage'` → still succeeds. |
| `0053_partial_index_exists_with_correct_predicate` | `pg_indexes` query confirms `idx_audit_outbox_import_genspark` present with `WHERE reservation_source = 'import_genspark'`. |
| `0053_in_migration_inventory_toml` | `grep -q 0053_audit_outbox_import_genspark services/canonical_ingest/migration_inventory.toml`. |

## 4. Cargo / feature-gating tests

| Test | Asserts |
|------|---------|
| `default_build_has_no_reqwest_dep` | `cargo tree -p spendguard-importer-genspark -e=normal` output contains no `reqwest` and no `hyper-tls`. |
| `default_build_has_no_secrecy_dep` | Same: no `secrecy` crate in default-features dep tree. |
| `live_feature_pulls_reqwest` | `cargo tree -p spendguard-importer-genspark --features live -e=normal` includes `reqwest`. |
| `live_feature_pulls_secrecy` | Same: includes `secrecy`. |
| `bin_runs_without_live_feature_in_fixture_mode` | `cargo run --bin spendguard-importer-genspark -- --fixture … --window-from … --window-to … --database-url … --price …` exits 0. |
| `bin_rejects_workspace_arg_without_live_feature` | Without `--features live`, `cargo run … --workspace foo` returns a clear error. |
| `package_is_publish_false` | `cargo metadata` shows `publish = false` for `spendguard-importer-genspark`. |

## 5. Demo regression gates

| ID | Command | Asserts |
|----|---------|---------|
| `T5.1` | `make -C deploy/demo demo-verify-import-genspark-fixture` exits 0 | Imports the Plus fixture; SQL verifier confirms ≥ 2 audit rows, non-zero priced aggregate, zero ledger rows. |
| `T5.2` | `make -C deploy/demo demo-clean` removes D16 artefacts | Genspark importer-written rows are purged after `demo-clean`. |
| `T5.3` | `make -C deploy/demo demo-verify-litellm-real` exits 0 (regression) | Pre-existing BYOK demo still passes (D16 didn't break ledger path). |
| `T5.4` | `make -C deploy/demo demo-verify-pricing` exits 0 (regression) | Pricing-table loader still works for unrelated services. |

## 6. Negative / red-team tests

| Test | Asserts |
|------|---------|
| `parse_handles_giant_workspace_id` | 4 KiB `workspace_id` string → parses, but row insertion truncates to the column's declared length (assumes `tenant_id` text limit). |
| `parse_handles_negative_credits_consumed` | `credits_consumed: -500` → parses, `import_record_to_audit_row` returns `amount_micro_usd = -999500` (saturating, no panic), reviewer flag in `T15` (review-standards). |
| `parse_handles_credits_overflow_i64` | `credits_consumed: i64::MAX` → conversion saturates, no panic. |
| `live_client_token_with_whitespace_treated_as_empty` | `GENSPARK_API_TOKEN = "   "` → `Err` (whitespace-only after `trim` check). |
| `live_client_token_with_embedded_newline_returns_err_or_safe` | Token containing `\n` → either rejected OR stored as-is but never logged. Verified by tracing capture. |
| `fixture_with_null_byte_in_workspace_id` | Embedded `\0` in JSON string → serde rejects with parse error, no panic. |
| `fixture_with_invalid_rfc3339_timestamps` | Bad `window_end` → `Err`, no panic. |

## 7. Performance gates

| Test | Asserts |
|------|---------|
| `import_window_from_fixture_p99_under_50ms` | 100-record fixture → end-to-end (parse + price + write) p99 < 50 ms against an in-process PG. Importer is a batch worker, not a hot path; 50 ms per 100 rows is the budget. |
| `price_table_load_under_5ms` | `GensparkPriceTable::load` on the committed config → < 5 ms (TOML parse). |
| `record_to_row_under_10us` | 10k iterations of `import_record_to_audit_row` → p99 < 10 µs (pure compute). |

## 8. Test inventory summary

- Unit tests: ~45 across `price.rs`, `audit.rs`, `record.rs`, `live.rs` (gated).
- Fixture-driven integration: 13 + 3 provenance gates.
- Migration: 11.
- Cargo / feature-gating: 7.
- Demo regression: 4.
- Negative / red-team: 7.
- Performance: 3.

Total ~93 tests. Every gate runs in `cargo test --manifest-path services/importer_genspark/Cargo.toml` (default features) + `cargo test --features live` (gating only, no live HTTP) + `make -C deploy/demo demo-verify-import-genspark-fixture`. No CI gate requires a live `GENSPARK_API_TOKEN` or any outbound network call.
