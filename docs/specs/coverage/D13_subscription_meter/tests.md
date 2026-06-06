# D13 — Tests

Companion to [`design.md`](design.md) and [`implementation.md`](implementation.md). Defines unit coverage, fixture-driven integration coverage, demo-mode regression coverage, and the hard-cap CLI-behaviour assertion.

## 1. Unit tests

### 1.1 `subscription.rs` — classifier

| Test | Asserts |
|------|---------|
| `classify_claude_code_pro_oauth_token` | `Bearer sk-ant-oat01-AAAA…` + `User-Agent: claude-cli/1.4.0` + Anthropic provider → `ClaudeCodePro`. |
| `classify_claude_byok_api_key` | `Bearer sk-ant-api03-…` + `User-Agent: claude-cli/1.4.0` → `Byok` (key prefix wins per design §4.1). |
| `classify_claude_byok_with_python_sdk` | `Bearer sk-ant-api03-…` + `User-Agent: anthropic-python/0.42` → `Byok`. |
| `classify_codex_chatgpt_jwt` | `Bearer eyJ…` (JWT) + `User-Agent: codex_cli_rs/0.32` + OpenAI provider → `CodexChatGpt`. |
| `classify_codex_byok_project_key` | `Bearer sk-proj-…` + `User-Agent: codex_cli_rs/0.32` → `Byok`. |
| `classify_openai_python_byok` | `Bearer sk-…` + `User-Agent: OpenAI/Python 1.50.0` → `Byok`. |
| `classify_ambiguous_ua_only` | Subscription-shaped User-Agent + BYOK key prefix → `Byok` (UA is forgeable). |
| `classify_ambiguous_key_only` | OAuth-shaped key + non-CLI User-Agent → `Byok` (paranoid default). |
| `classify_no_authorization_header` | Missing `Authorization` → `Byok` (proxy will fail later; classifier doesn't gate on auth presence). |
| `classify_non_bearer_scheme` | `Authorization: Basic …` → `Byok`. |
| `classify_never_logs_full_token` | After classification, the unit test inspects captured `tracing` events: no event field contains more than 13 chars of the token. Enforced via `tracing-test` subscriber capture. |
| `classify_prefix_extractor_is_constant_time_safe` | Bench harness (`#[ignore]`) — prefix extraction does not branch on token chars beyond the first 13. |

### 1.2 `subscription_meter.rs` — estimate

| Test | Asserts |
|------|---------|
| `meter_estimate_uses_input_tokens_from_tokenizer` | Mock tokenizer returns 1500 input tokens → `MeterEstimate.input_tokens == 1500`. |
| `meter_estimate_predicted_output_falls_back_to_4096` | Body without `max_tokens` → `estimated_output_tokens == 4096`. |
| `meter_estimate_respects_max_tokens_in_body` | `{"max_tokens": 1024}` → `estimated_output_tokens == 1024`. |
| `meter_estimate_uses_pricing_table_retail` | Mock pricing snapshot with `claude-3-5-sonnet input=$3/M, output=$15/M`, 1000 input + 1000 output → `estimated_amount_micro_usd == 18_000`. |
| `meter_estimate_pricing_version_propagates` | Pricing snapshot version `demo-pricing-v3` → `MeterEstimate.pricing_version == "demo-pricing-v3"`. |
| `meter_estimate_unknown_model_returns_err` | Pricing table has no entry → `Err(MeterError::PricingMissing)`, no panic. |
| `meter_estimate_never_calls_sidecar` | Mock sidecar client wrapped in `panic_on_call` — `meter_only_estimate` MUST NOT invoke it. |
| `meter_estimate_never_writes_ledger` | Test holds a Postgres txn open: after `meter_only_estimate`, `ledger_entries` rowcount unchanged. |

### 1.3 `subscription_meter.rs` — cap evaluation

| Test | Asserts |
|------|---------|
| `evaluate_cap_meter_mode_always_pass` | `mode = Meter` → `CapDecision::Pass` regardless of used / threshold. |
| `evaluate_cap_soft_below_threshold_pass` | `used + this < threshold` → `Pass`. |
| `evaluate_cap_soft_at_threshold_alerts` | `used + this >= threshold` → `SoftCapAlert(payload)` with correct used + threshold. |
| `evaluate_cap_hard_above_threshold_blocks` | `mode = HardCap` + over threshold → `HardCapBlock(block)`. |
| `evaluate_cap_hard_retry_after_within_window` | `Block429.retry_after_seconds <= window_remaining_seconds`. |
| `evaluate_cap_no_cap_configured_passes` | `fetch_cap` returns `None` → `Pass` even in `HardCap` mode. |
| `evaluate_cap_window_anchor_utc_calendar_month` | `threshold_window = P1M` + `window_anchor_utc = NULL` → window starts at `date_trunc('month', now() AT TIME ZONE 'UTC')`. |
| `evaluate_cap_window_explicit_anchor` | Operator-pinned `window_anchor_utc = 2026-06-01T00:00:00Z` → that exact moment is window start. |
| `evaluate_cap_used_in_window_only_counts_subscription_rows` | Mock cap store: ledger rows in window don't count; only `audit_outbox` rows with `reservation_source = 'subscription_meter'` do. |

### 1.4 `routing.rs` — Codex/ChatGPT row

| Test | Asserts |
|------|---------|
| `routes_codex_chatgpt_responses` | `route("/backend-api/codex/responses")` → `ProviderKind::OpenAi`, `RequestShape::OpenAiResponses`. |
| `codex_chatgpt_path_does_not_collide_with_openai_v1_responses` | `/v1/responses` and `/backend-api/codex/responses` route distinctly. |
| `codex_chatgpt_upstream_url_is_chatgpt_dot_com` | `upstream_url_for(…)` contains `chatgpt.com`, NOT `api.openai.com`. |
| `codex_chatgpt_tokenizer_kind_is_openai` | Tokenizer dispatch uses OpenAI BPE (Codex uses o200k-class encodings). |

### 1.5 Sidecar branch tests (`sidecar/src/decision/transaction.rs`)

| Test | Asserts |
|------|---------|
| `subscription_meter_skips_ledger_write` | `DecisionRequest.reservation_source = SUBSCRIPTION_METER` → no row in `ledger_entries` after txn commit. |
| `subscription_meter_writes_audit_outbox` | Same input → exactly one row in `audit_outbox` with `reservation_source = 'subscription_meter'`. |
| `byok_default_still_writes_ledger` | `reservation_source = UNSPECIFIED` (legacy clients) → behaves identically to `BYOK` and writes ledger. |
| `byok_explicit_writes_ledger` | `reservation_source = BYOK` → writes ledger (existing regression baseline). |
| `subscription_meter_does_not_call_reserve_proc` | Mock pg shim: `reserve_v2` PL/pgSQL function never invoked. |

### 1.6 Importer stub contract (`importer_anthropic` + `importer_openai`)

| Test | Asserts |
|------|---------|
| `import_record_to_audit_row_sets_subscription_meter` | Output row has `reservation_source = "subscription_meter"`. |
| `import_record_to_audit_row_sets_import_source_anthropic` | Anthropic importer → `import_source = "anthropic_console_usage"`. |
| `import_record_to_audit_row_sets_import_source_openai` | OpenAI importer → `import_source = "openai_admin_usage"`. |
| `import_record_amount_conversion_micro_usd` | `usd_amount = 0.0035` → `amount_micro_usd = 3500`, no float drift > ±1 micro-USD. |
| `import_record_schema_matches_pg_check_constraint` | Round-trip: insert the generated row into a test PG instance with migration 0046 applied — no CHECK constraint violation. |
| `live_feature_gated_off_by_default` | `cargo check -p spendguard-importer-anthropic` does not pull `reqwest` or any HTTP client (stub mode only). |

## 2. Fixture-driven integration tests

`services/egress_proxy/tests/subscription_meter_e2e.rs` — runs each HAR fixture through the proxy with a stub upstream (returns canned 200 responses) and asserts the audit row.

| Test | Fixture | Asserts |
|------|---------|---------|
| `claude_code_pro_session_meters_correctly` | `claude_code_pro_session.har` | Classifier → `ClaudeCodePro`. ≥ 1 audit row written with `reservation_source = 'subscription_meter'`. `ledger_entries` rowcount unchanged. `amount_micro_usd > 0`. |
| `codex_chatgpt_plus_session_meters_correctly` | `codex_chatgpt_plus_session.har` | Same shape; `model` field populated from response. |
| `byok_anthropic_session_uses_ledger` | `byok_anthropic.har` | Audit row has `reservation_source = 'byok'`. Ledger row written. (Regression: confirms D13 doesn't break BYOK.) |
| `byok_openai_session_uses_ledger` | `byok_openai.har` | Same. |
| `ambiguous_cli_byok_uses_ledger` | `ambiguous_cli_byok.har` | Classifier defends against UA-only signal: `Byok`. |
| `mixed_session_byok_then_subscription` | Two requests back-to-back, one BYOK Anthropic + one Claude Code Pro | Both rows written, distinct `reservation_source` values, ledger has exactly the BYOK entry. |
| `claude_code_pro_with_invalid_token_falls_to_byok` | HAR with malformed OAuth prefix | Classifier degrades safely to `Byok` (proxy then forwards; vendor 401s). |

### 2.1 Fixture provenance

`services/egress_proxy/tests/fixtures/subscription/PROVENANCE.md` lists every fixture with:

- Capture date
- Capturing operator (initials)
- Source tool + version (`Claude Code 1.4.0`, `codex_cli_rs 0.32.1`)
- Redaction script: `scripts/redact_har.py --har <in> --out <out>` (SHA-256 pinned)
- SHA-256 of the **original** `Authorization` header for audit only
- Confirmation that no PII / customer prompt content survives redaction

Redaction script replaces:
- Every `Authorization` header value → `FAKE_OAUTH_TOKEN_<8_random_hex>`
- Every JWT body claim → fixed test value
- Every `messages[].content` string → `"<redacted prompt>"`
- Every response `content` string → `"<redacted response>"` while preserving `usage.input_tokens` + `usage.output_tokens` so the meter calculation is deterministic

## 3. Hard-cap synthetic 429 integration tests

`services/egress_proxy/tests/hard_cap_synthetic_429.rs`:

| Test | Asserts |
|------|---------|
| `hard_cap_returns_429_before_upstream_call` | Mock upstream wrapped in `panic_on_request`. Hard-cap triggers. Proxy returns 429 without calling upstream. |
| `hard_cap_429_body_matches_anthropic_shape` | Response body JSON has `error.type == "rate_limit_exceeded"` (matches Anthropic's vendor 429 shape so Claude Code CLI handles it identically). |
| `hard_cap_retry_after_header_present_and_positive` | `Retry-After` header parses as integer > 0. |
| `hard_cap_retry_after_caps_at_window_remaining` | If window resets in 3600s, `Retry-After <= 3600`. |
| `hard_cap_writes_stop_run_audit_row` | Audit row has `decision = STOP_RUN_PROJECTION`, `reason_code = "subscription_cap_exceeded"`. |
| `hard_cap_does_not_block_other_tenant` | Tenant A hits cap; tenant B's call same instant passes through. |
| `hard_cap_resets_after_window_rollover` | Mock clock advanced past window end → next call passes through, new audit row written for new window. |

### 3.1 CLI exit-code assertion (offline fixture-based)

We do **not** spawn real Claude Code or Codex CLI binaries in CI (they require live OAuth flow). Instead, the test asserts that the 429 response shape matches the documented CLI error-handling contract:

| CLI | Documented behaviour on 429 | Asserted by |
|-----|-----------------------------|-------------|
| Claude Code | Surfaces "rate limit exceeded" to operator, exits 1 | Response shape unit test (`hard_cap_429_body_matches_anthropic_shape`) + recorded CLI behaviour in `docs/specs/coverage/D13_subscription_meter/cli-behaviour-on-429.md` |
| Codex CLI | Same, exits 1 | Same |

A separate gated test (`#[ignore] #[cfg(feature = "live-cli")]`) runs the actual binaries for operators who choose to validate manually; this is NOT a merge gate.

## 4. Soft-cap alert dispatch tests

`services/egress_proxy/tests/soft_cap_alert.rs`:

| Test | Asserts |
|------|---------|
| `soft_cap_emits_slack_payload_when_configured` | `SLACK_WEBHOOK_URL` set + threshold crossed → exactly one POST to mock Slack URL with payload `{text, attachments[0].fields[*]}` containing tenant_id, used_usd, threshold_usd. |
| `soft_cap_emits_pagerduty_event_when_configured` | `PAGERDUTY_INTEGRATION_KEY` set + threshold crossed → exactly one POST to PagerDuty Events API v2 with `event_action="trigger"`. |
| `soft_cap_writes_stderr_warning_to_proxy_log` | After threshold crossed, the proxy's stderr emits a `WARN`-level event with `kind="subscription_soft_cap"`. |
| `soft_cap_does_not_block_request` | Mock upstream IS called; response IS forwarded. |
| `soft_cap_alert_is_rate_limited` | Two requests crossing threshold within 60s → exactly one Slack POST (cooldown). |
| `soft_cap_alert_resets_after_cooldown` | Mock clock + 61s → second alert fires. |

## 5. Demo-mode regression tests

| ID | Command | Asserts |
|----|---------|---------|
| `T5.1` | `make -C deploy/demo demo-verify-subscription-meter-claude-code` exits 0 | Replay Claude Code HAR → meter row written, ledger unchanged. |
| `T5.2` | `make -C deploy/demo demo-verify-subscription-meter-codex` exits 0 | Same for Codex. |
| `T5.3` | `make -C deploy/demo demo-verify-subscription-hard-cap` exits 0 | Set mode=`hard_cap`, threshold=$0.00, fixture replay → proxy returns 429, no upstream call, audit row decision=STOP_RUN. |
| `T5.4` | `make -C deploy/demo demo-verify-litellm-real` exits 0 (regression) | Pre-existing BYOK demo still passes (D13 didn't break ledger path). |
| `T5.5` | `make -C deploy/demo demo-verify-pricing` exits 0 (regression) | Pricing table still works for meter mode. |

## 6. Schema migration tests

| Test | Asserts |
|------|---------|
| `0044_apply_and_rollback_idempotent` | Apply migration twice → no error. Rollback removes `reservation_source` column cleanly. |
| `0044_existing_rows_default_to_byok` | Pre-migration row + post-migration query → `reservation_source = 'byok'`. |
| `0044_check_constraint_rejects_invalid_value` | `INSERT … reservation_source = 'wat'` → SQLSTATE `23514` (CHECK violation). |
| `0044_partial_index_exists` | `pg_indexes` query confirms `idx_audit_outbox_subscription_meter` present with `WHERE reservation_source = 'subscription_meter'` predicate. |
| `0045_rls_isolation_prevents_cross_tenant_read` | Tenant A session cannot SELECT tenant B's cap row. |
| `0045_threshold_window_check_rejects_invalid_iso` | `INSERT … threshold_window = 'P2D'` → CHECK violation. |
| `0046_import_source_check_constraint` | `INSERT … import_source = 'wat'` → CHECK violation. |
| `0046_import_source_nullable_for_live_rows` | `INSERT … import_source = NULL` accepted (live proxy / sidecar path). |

## 7. Negative / red-team tests

| Test | Asserts |
|------|---------|
| `classify_handles_authorization_with_null_byte` | Embedded `\0` in Authorization → classifier returns `Byok`, no panic, full token never reaches a log. |
| `classify_handles_giant_user_agent` | 64 KiB User-Agent → classifier still returns in < 100µs, no allocation > 1 KiB. |
| `meter_estimate_rejects_negative_tokens_from_tokenizer` | Mock tokenizer returns `input_tokens = -1` (sentinel) → `Err(MeterError::TokenCountInvalid)`. |
| `evaluate_cap_handles_pricing_overflow` | `estimated_amount_micro_usd = i64::MAX` → saturating arithmetic, no panic. |
| `hard_cap_does_not_leak_other_tenant_usage_in_429` | Response body never contains another tenant's used/threshold figures. |
| `soft_cap_slack_payload_redacts_oauth_token` | Slack payload includes tenant_id but NEVER the inbound `Authorization` value. |

## 8. Performance gates

| Test | Asserts |
|------|---------|
| `classify_p99_under_50us` | 10k classification iterations → p99 < 50 µs (it's pure header inspection). |
| `meter_only_estimate_p99_under_5ms_fallback_path` | Mirrors `decision::estimate_call_cost_p99_under_5ms_fallback_path`. |
| `hard_cap_short_circuit_p99_under_2ms` | When cap triggers, total proxy-side latency p99 < 2 ms (no upstream RTT). |

## 9. Test inventory summary

- Unit tests: ~55 across `subscription.rs`, `subscription_meter.rs`, `subscription_cap_store.rs`, routing addition, sidecar branch.
- Fixture-driven integration: 7 HAR-replay tests.
- Hard-cap: 7 + 1 ignored live-CLI gate.
- Soft-cap alerts: 6.
- Demo regression: 5.
- Migration: 8.
- Negative / red-team: 6.
- Performance: 3.

Total ~97 tests. None require live OAuth credentials or live vendor API access. Every gate runs in `cargo test` + `make -C deploy/demo demo-verify-*`.
