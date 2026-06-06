# D15 — Tests

Companion to [`design.md`](design.md) and [`implementation.md`](implementation.md). Defines unit, contract, fixture-driven, live-mock, demo, migration, and negative-path coverage.

## 1. Unit tests

### 1.1 `pricing.rs` — credit → micro-USD

| Test | Asserts |
|------|---------|
| `team_plan_credit_cost_is_20526_micro_usd` | Price table loads `team_plan.credit_cost_micro_usd == 20_526`. |
| `team_plan_47_credits_equals_964_722_micro_usd` | `47 * 20_526 == 964_722`. |
| `enterprise_credit_cost_defaults_to_zero` | `enterprise.credit_cost_micro_usd == 0` (operator must override via config). |
| `enterprise_byok_credit_cost_is_zero` | BYOK tier costs nothing — customer pays the LLM provider direct. |
| `credit_to_usd_micros_zero_credits_is_zero` | `credits_consumed = 0` → `Ok(0)` (cancelled session is a valid row). |
| `credit_to_usd_micros_unknown_tier_returns_err` | Synthetic `Tier::Unknown`-equivalent path → `Err(MeterError::UnknownTier)` — covered via the loader path. |
| `credit_to_usd_micros_overflow_saturates` | `credits_consumed = i64::MAX`, `per_credit = 1` → saturating mul returns `i64::MAX`, no panic. |
| `credit_to_usd_micros_negative_amount_rejected` | If saturating mul would overflow into negative, return `Err(MeterError::NegativeAmount)`. |
| `price_table_load_embedded_succeeds` | `PriceTable::load_embedded()` doesn't panic; all three tier keys present. |
| `price_table_unknown_tier_key_returns_err` | Asking for `Tier::Enterprise` when the toml omits `enterprise` → `MeterError::UnknownTier`. |

### 1.2 `fixture.rs` — loader + validation

| Test | Asserts |
|------|---------|
| `load_fixture_parses_all_8_sessions` | The shipped `manus_usage.json` yields 8 `ImportRecord`s. |
| `load_fixture_in_progress_status_preserved` | `status = "in_progress"` → `SessionStatus::InProgress` (loader does NOT drop it; the downstream demo path is what skips). |
| `load_fixture_unknown_tier_returns_err` | Synthetic fixture with `"tier": "wat"` → `Err(ImporterError::UnknownTier)`. |
| `load_fixture_unknown_status_returns_err` | Synthetic fixture with `"status": "wat"` → `Err(ImporterError::UnknownStatus)`. |
| `load_fixture_negative_credits_rejected` | `"credits_consumed": -5` → `Err(ImporterError::NegativeCredits)`. |
| `load_fixture_missing_file_returns_err` | Path that doesn't exist → `Err(ImporterError::Io(_))`. |
| `load_fixture_malformed_json_returns_err` | File contents `not json` → `Err(ImporterError::Parse(_))`. |
| `load_fixture_ignores_extra_fields` | Synthetic fixture with an extra `"vendor_internal_field": "x"` → load succeeds (forward-compat). |
| `load_fixture_completed_at_before_started_at_loads_anyway` | Loader does not enforce temporal ordering — vendor anomalies pass through to the audit row, surfaced on the dashboard, not at import. |
| `load_fixture_workspace_id_preserved_verbatim` | `ws_FAKE_…` lands intact on `ImportRecord.workspace_id`. |

### 1.3 `audit.rs` — `import_record_to_audit_row`

| Test | Asserts |
|------|---------|
| `audit_row_reservation_source_is_import_manus` | Output has `reservation_source == "import_manus"`. |
| `audit_row_import_source_is_manus_admin_usage` | Output has `import_source == Some("manus_admin_usage")`. |
| `audit_row_model_is_synthetic_session_credit` | `model == "manus.session/credit"`. |
| `audit_row_input_output_tokens_are_zero` | Manus exposes no token detail → both zero. |
| `audit_row_amount_matches_credit_to_usd_micros` | `amount_micro_usd == credit_to_usd_micros(rec, table)?`. |
| `audit_row_occurred_at_is_window_end` | `occurred_at == window_end` (matches Anthropic-importer convention from D13 §5). |
| `audit_row_dedupe_key_includes_session_id` | `dedupe_key == Some("manus:ses_FAKE_…")`. |
| `audit_row_tenant_id_is_workspace_id` | Maps `workspace_id → tenant_id` 1:1. |
| `audit_row_unknown_tier_returns_err_no_panic` | Forced unknown tier via raw `ImportRecord` constructor → `Err`, no row produced. |
| `audit_row_pure_function` | Two consecutive calls with identical input produce byte-identical output (no clocks, no UUIDs, no IO). |

### 1.4 `live.rs` — feature-gated env handling

`#[cfg(feature = "live")]` block:

| Test | Asserts |
|------|---------|
| `from_env_returns_missing_token_when_unset` | `MANUS_API_TOKEN` unset → `Err(ImporterError::MissingToken)`. |
| `from_env_returns_missing_token_when_empty` | `MANUS_API_TOKEN=""` → same. |
| `from_env_uses_default_base_url_when_unset` | `MANUS_API_BASE_URL` unset → client targets `https://api.manus.ai`. |
| `from_env_respects_override_base_url` | `MANUS_API_BASE_URL=http://localhost:1234` → client targets that URL (covers mock-server tests + on-prem proxy story). |

## 2. Contract tests (`tests/contract.rs`)

These run against a real Postgres instance with migrations applied — required to prove the importer's output round-trips through the CHECK constraints.

| Test | Asserts |
|------|---------|
| `audit_row_round_trips_through_migration_0047` | Generated `AuditRow` → `INSERT INTO audit_outbox` → no CHECK violation on `reservation_source = 'import_manus'`. |
| `audit_row_round_trips_through_migration_0048` | Same insert → no CHECK violation on `import_source = 'manus_admin_usage'`. |
| `audit_row_with_invalid_reservation_source_rejected` | Synthetic row with `reservation_source = 'wat'` → SQLSTATE `23514`. |
| `audit_row_with_invalid_import_source_rejected` | Same for `import_source = 'wat'`. |
| `partial_index_idx_audit_outbox_import_manus_exists` | `pg_indexes` query confirms the partial index from mig 0047 is present with the correct predicate. |
| `dedupe_key_uniqueness_prevents_double_import` | Two imports of the same fixture → second `INSERT` upserts/no-ops; total row count stays at 7. (Enforced via `dedupe_key` column / existing canonical_ingest convention.) |

## 3. Fixture-driven integration test (`tests/fixture_e2e.rs`)

This is the **headline merge gate** matching the deliverable prompt's acceptance criterion.

| Test | Asserts |
|------|---------|
| `fixture_import_emits_seven_audit_events` | Load `manus_usage.json` → run the full pipeline (`load_fixture → import_record_to_audit_row → append_audit_outbox`) → 7 rows in `audit_outbox` (in_progress dropped at demo layer; this test uses the demo policy). |
| `fixture_import_each_row_tagged_import_manus` | Every emitted row has `reservation_source = 'import_manus'`. |
| `fixture_import_each_row_tagged_manus_admin_usage` | Every emitted row has `import_source = 'manus_admin_usage'`. |
| `fixture_import_no_ledger_write` | `ledger_entries` count unchanged before/after the run. |
| `fixture_import_team_plan_amounts_match_pricing` | Sum of team_plan rows == `1010 * 20_526 = 20_731_260` micro-USD. |
| `fixture_import_enterprise_byok_amount_is_zero` | BYOK row has `amount_micro_usd = 0`. |
| `fixture_import_idempotent_across_two_runs` | Run twice → `audit_outbox` row count is 7, not 14 (dedupe via `dedupe_key`). |
| `fixture_import_cloudevent_type_is_manus_credit` | `outbox_forwarder` (or its in-process test double) observes type `spendguard.audit.import.manus_credit` on every emitted CloudEvent. |

## 4. Live-mode mock-server tests (`tests/live_mock.rs`, `#[cfg(feature = "live")]`)

Uses `httpmock` (dev-dep) to fake the Manus admin REST surface. Default `cargo test -p spendguard-importer-manus` skips these (correct: `live` feature is off). `cargo test -p spendguard-importer-manus --features live` runs them.

| Test | Asserts |
|------|---------|
| `live_poll_usage_with_token_succeeds` | Mock server returns `manus_usage.json` body. `LiveClient::poll_usage` returns 8 records. |
| `live_poll_usage_sends_bearer_token` | `httpmock` asserts the `Authorization: Bearer <token>` header arrived. |
| `live_poll_usage_handles_cursor_pagination` | Mock returns two pages (`next_cursor` set on first response, null on second). Client makes 2 requests, returns combined records. |
| `live_poll_usage_skips_malformed_records_with_warn` | Mock returns one valid + one tier-`wat` record. Client returns 1 record + `tracing` captures a WARN with `error = UnknownTier`. |
| `live_poll_usage_http_500_returns_err` | Mock returns 500 → client returns `Err(_)`, no `unwrap` panic. |
| `live_poll_usage_http_401_returns_err_with_redacted_token` | Mock returns 401 → error message does NOT contain the raw token (greppable assert). |
| `live_poll_usage_timeout_returns_err` | Mock delays beyond the configured 30s timeout (we set a smaller timeout in the test) → `Err(_)`. |
| `live_poll_usage_user_agent_set` | Mock asserts `User-Agent` starts with `spendguard-importer-manus/`. |

## 5. Demo-mode regression tests

| ID | Command | Asserts |
|----|---------|---------|
| `T5.1` | `make -C deploy/demo demo-import-manus-fixture` exits 0 | Replay `manus_usage.json` → 7 audit rows landed with `reservation_source = 'import_manus'`, no ledger writes, team_plan total = `20_731_260` micro-USD. |
| `T5.2` | `make -C deploy/demo demo-import-manus-fixture` exits 0 when run twice in a row | Idempotency: second run does not double rows. |
| `T5.3` | `make -C deploy/demo demo-verify-litellm-real` exits 0 (regression) | Pre-existing BYOK demo still passes — D15 didn't break the egress proxy path. |
| `T5.4` | `make -C deploy/demo demo-verify-subscription-meter-claude-code` exits 0 (regression) | D13 demo still passes — D15's CHECK extension did not break the subscription_meter path. |
| `T5.5` | `make -C deploy/demo demo-clean` clears D15-specific rows | After clean, `import_manus` audit rows purged. |

## 6. Schema migration tests

| Test | Asserts |
|------|---------|
| `0047_apply_clean` | Migration 0047 applies cleanly against a DB that has all preceding migrations including D13 0044. |
| `0047_preserves_pre_existing_rows` | Pre-0047 rows with `reservation_source = 'byok'` or `'subscription_meter'` are still queryable; nothing dropped. |
| `0047_rejects_unknown_reservation_source` | `INSERT … reservation_source = 'wat'` → SQLSTATE `23514`. |
| `0047_partial_index_exists` | `pg_indexes` shows `idx_audit_outbox_import_manus` with `WHERE reservation_source = 'import_manus'`. |
| `0047_down_migration_round_trips` | Apply 0047, apply 0047_down, run the D13 0044 negative-test (`'wat'` rejected) → still works. |
| `0048_apply_clean` | Migration 0048 applies cleanly. |
| `0048_preserves_pre_existing_rows` | Pre-0048 rows with `import_source IS NULL` or `'anthropic_console_usage'` / `'openai_admin_usage'` still queryable. |
| `0048_rejects_unknown_import_source` | `INSERT … import_source = 'wat'` → SQLSTATE `23514`. |
| `0048_null_import_source_still_accepted` | `INSERT … import_source = NULL` accepted (live proxy / sidecar path unchanged). |
| `0048_down_migration_round_trips` | Apply 0048, apply 0048_down → D13's narrower CHECK is back. |
| `migration_inventory_pins_0047_0048` | `migration_inventory.toml` lists both with SHA-256 pins. |

## 7. Negative / red-team tests

| Test | Asserts |
|------|---------|
| `fixture_with_giant_credits_does_not_overflow` | Synthetic fixture with `credits_consumed = 9_223_372_036_854_775_807` → saturating mul + `NegativeAmount` guard catches it, no panic. |
| `fixture_with_quotes_in_workspace_id_safe` | Workspace ID containing `'`, `"`, ` `, `;DROP TABLE` round-trips via parameterised SQL — no injection. |
| `live_mode_does_not_log_full_token` | All `tracing` events captured during a successful poll have no field containing the raw token (length > 8 chars of the bearer body). |
| `live_mode_does_not_log_full_token_on_error` | All `tracing` events captured during a 401 error similarly redact the token. |
| `live_mode_does_not_panic_on_redirect_loop` | Mock returns 302 → 302 → 302; reqwest's default redirect limit kicks in, returns `Err`, not a stack overflow. |
| `default_build_has_no_reqwest_in_cargo_tree` | `cargo tree -p spendguard-importer-manus -e=normal` returns no `reqwest` line. (Verified in acceptance, also asserted here as a code-level test via `compile_error!` if a stray non-optional reqwest sneaks in.) |
| `audit_row_tenant_id_never_empty_for_completed_session` | Fixture invariant: every completed session has a non-empty `workspace_id`; loader rejects an empty value at validation time. |
| `audit_row_amount_never_negative` | Property-style: 200 random `ImportRecord`s (positive credits, valid tier) → `amount_micro_usd >= 0` always. |

## 8. Performance gates

| Test | Asserts |
|------|---------|
| `load_fixture_8_sessions_under_10ms` | `load_fixture` on the shipped JSON returns in p99 < 10ms (single-threaded, release build). |
| `credit_to_usd_micros_under_100ns` | Pure-math hot path — p99 < 100ns over 100k iterations. |
| `import_record_to_audit_row_under_500ns` | p99 < 500ns over 100k iterations (no allocation beyond String clones already in `ImportRecord`). |

These are `#[ignore]`'d by default; run with `cargo test --release -- --ignored`.

## 9. Test inventory summary

- Unit (pricing + fixture + audit + live env): ~36
- Contract (migration round-trip): 6
- Fixture-driven integration: 8
- Live-mode mock-server: 8 (gated on `--features live`)
- Demo regression: 5
- Migration: 11
- Negative / red-team: 8
- Performance: 3

Total ~85 tests. None require live network access by default. Every default-features gate runs in `cargo test -p spendguard-importer-manus` + `make -C deploy/demo demo-import-manus-fixture`. Live-mode tests are opt-in via `--features live` and use `httpmock`, not the real Manus API.
