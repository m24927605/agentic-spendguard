# D14 — Tests

Companion to [`design.md`](design.md) and [`implementation.md`](implementation.md). Defines unit coverage, fixture round-trip, CloudEvent golden, live-client `wiremock` coverage (gated on `live` feature), demo-mode regression, and migration tests.

## 1. Unit tests

### 1.1 `acu_price_table.rs` — loader + conversion

| Test | Asserts |
|------|---------|
| `price_table_loads_from_embedded_asset` | `AcuPriceTable::load_from_embedded()` succeeds; `pricing_version == "devin-acu-v1-2026-06"`; rates non-empty. |
| `price_table_team_plan_round_trip` | `lookup("team").usd_per_acu == Some(2.25)`. |
| `price_table_enterprise_plan_has_null_rate` | `lookup("enterprise").usd_per_acu == None` AND `note.is_some()`. |
| `price_table_unknown_plan_returns_err` | `lookup("unknown")` returns `PriceLookupError::PlanNotFound("unknown")`, no panic. |
| `acu_to_micro_usd_round_trip` | `acu = 12.5` × `usd = 2.25` → `28_125_000` micro-USD; round-trip lossless within ±1 micro-USD. |
| `acu_to_micro_usd_handles_zero_acu` | `acu = 0.0` → `0` micro-USD. |
| `acu_to_micro_usd_rounds_half_to_even` | `acu = 0.4444444` × `usd = 2.25` → consistent banker's-rounded output across re-runs. |

### 1.2 `import_record.rs` — `import_record_to_audit_row`

| Test | Asserts |
|------|---------|
| `import_record_to_audit_row_sets_subscription_meter` | Output `AuditRow.reservation_source == "subscription_meter"`. |
| `import_record_to_audit_row_sets_import_source_devin_team_api` | Output `AuditRow.import_source == Some("devin_team_api")`. |
| `import_record_to_audit_row_amount_conversion` | `acu = 12.5`, plan `team` → `amount_micro_usd == Some(28_125_000)`. |
| `import_record_to_audit_row_enterprise_plan_nulls_amount` | Plan `enterprise` → `amount_micro_usd == None` AND `reason_code == Some("devin_enterprise_negotiated_rate")`. |
| `import_record_to_audit_row_stamps_pricing_version` | Output `pricing_version == Some("devin-acu-v1-2026-06")`. |
| `import_record_to_audit_row_model_field_encodes_plan` | Output `model == "devin/acu/team"` (allows dashboard split by plan). |
| `import_record_to_audit_row_is_pure_no_io` | Function signature is `fn(&ImportRecord, &AcuPriceTable) -> Result<…>`; reviewer confirms no `async`, no global state. |
| `import_record_to_audit_row_propagates_window_end_as_occurred_at` | Output `occurred_at == rec.window_end` (canonical audit time). |
| `import_record_to_audit_row_unknown_plan_returns_err` | Plan not in table → `PriceLookupError::PlanNotFound`, never panics. |

### 1.3 `cloudevent_envelope.rs` — builder

| Test | Asserts |
|------|---------|
| `cloudevent_envelope_type_constant` | `event_type == "spendguard.audit.import.devin_acu"` — exact string. |
| `cloudevent_envelope_source_constant` | `source == "spendguard-importer-devin"`. |
| `cloudevent_envelope_schema_version` | `data.schema_version == "v1alpha1"`. |
| `cloudevent_envelope_subject_format` | `subject == "tenant/<tid>/devin/team/<dt>/session/<ds>"` — slash-separated, no trailing slash. |
| `cloudevent_envelope_id_is_uuidv7` | `id` parses as UUIDv7 (extracted timestamp within ±2s of build time). |
| `cloudevent_envelope_time_is_rfc3339_utc` | `time` parses + `tz_offset == 0`. |
| `cloudevent_envelope_fixture_mode_records_provenance` | When `ingestion_mode = Fixture`, `data.fixture_provenance_sha256` is `Some(<64 hex>)`. |
| `cloudevent_envelope_live_mode_omits_provenance` | When `ingestion_mode = Live`, `data.fixture_provenance_sha256` is `None`. |
| `cloudevent_envelope_enterprise_plan_nulls_amount_and_usd_per_acu` | Plan `enterprise` → both `data.amount_micro_usd` and `data.usd_per_acu` serialize as `null`. |
| `cloudevent_envelope_golden_v1alpha1` | Builds an envelope from a frozen `ImportRecord` (UUIDv7 + time injected for determinism), serializes to JSON, asserts byte-equality with `tests/golden/cloudevent_v1alpha1.json` committed file. |

### 1.4 `fixture_loader.rs`

| Test | Asserts |
|------|---------|
| `fixture_loader_reads_canonical_snapshot` | Loads `tests/fixtures/devin_usage.json` → returns ≥ 3 `ImportRecord`. |
| `fixture_loader_computes_sha256_once` | Hash is cached after construction; multiple `.records()` calls don't re-read the file. |
| `fixture_loader_records_carry_fixture_ingestion_mode` | Every returned record has `ingestion_mode == Fixture` AND `fixture_provenance_sha256 == Some(<hash>)`. |
| `fixture_loader_synthetic_ids_only` | Every record's `devin_team_id` matches `^TEAM_FIXTURE_\d{3}$`; `devin_session_id` matches `^SESSION_FIXTURE_\d{3}$`. |
| `fixture_loader_invalid_json_returns_err` | Malformed JSON → `Err`, no panic. |
| `fixture_loader_missing_file_returns_err` | Non-existent path → `Err(NotFound)`. |

### 1.5 Live-client `wiremock` tests (feature `live`)

`#[cfg(feature = "live")]` — all gated behind the feature flag, not run in default CI.

| Test | Asserts |
|------|---------|
| `live_client_missing_token_errors_clearly` | `DEVIN_API_TOKEN` unset → `DevinClient::from_env()` returns `LiveError::MissingToken` with the literal env-var name in the error message. |
| `live_client_fetch_team_usage_happy_path` | Wiremock GET `/api/v1/teams/T1/usage?start=…&end=…` returns canonical JSON → parses into `Vec<UsageRow>`. |
| `live_client_401_maps_to_unauthorized` | Mock returns 401 → `Err(LiveError::Unauthorized)`. |
| `live_client_403_maps_to_forbidden` | Mock returns 403 → `Err(LiveError::Forbidden)`. |
| `live_client_429_extracts_retry_after` | Mock returns 429 with `Retry-After: 60` → `Err(LiveError::RateLimited(60))`. |
| `live_client_5xx_maps_to_upstream` | Mock returns 503 → `Err(LiveError::Upstream(503))`. |
| `live_client_does_not_log_token` | After any call, `tracing-test` subscriber capture contains zero log fields with the bearer token value. |
| `live_client_uses_rustls_not_nativetls` | Build-time assert: `cargo tree -p spendguard-importer-devin --features live` shows `rustls` but NOT `native-tls` / `openssl-sys`. |

## 2. Fixture round-trip integration test

`services/importer_devin/tests/fixture_round_trip.rs`:

| Test | Asserts |
|------|---------|
| `fixture_to_audit_outbox_row_round_trip` | (a) Load `devin_usage.json`; (b) convert each `ImportRecord` to an `AuditRow`; (c) INSERT into a test PG instance with mig 0047 applied; (d) `SELECT … WHERE import_source = 'devin_team_api'` returns the same rowcount. No CHECK violation. |
| `fixture_to_audit_outbox_emits_cloudevent` | (a) Load fixture; (b) build envelope; (c) serialize to JSON; (d) downstream `canonical_ingest::parse_cloudevent` accepts it without error. |
| `fixture_round_trip_is_idempotent` | Same fixture → two runs → audit_outbox has the same rowcount (idempotency key dedups via canonical_ingest replay). |
| `fixture_enterprise_record_lands_with_null_amount` | Fixture includes one enterprise-plan record → corresponding `audit_outbox` row has `amount_micro_usd IS NULL` AND `reason_code = 'devin_enterprise_negotiated_rate'`. |
| `fixture_pricing_version_stamped` | Every `audit_outbox` row from the fixture has `pricing_version = 'devin-acu-v1-2026-06'`. |

## 3. Schema migration tests

`services/importer_devin/tests/pg_check_constraint.rs`:

| Test | Asserts |
|------|---------|
| `mig_0047_apply_and_rollback_idempotent` | Apply twice → no error. |
| `mig_0047_accepts_devin_team_api` | INSERT with `import_source = 'devin_team_api'` succeeds. |
| `mig_0047_still_accepts_d13_values` | INSERT with `import_source = 'anthropic_console_usage'` or `'openai_admin_usage'` still succeeds (D13 regression). |
| `mig_0047_rejects_unknown_value` | INSERT with `import_source = 'wat'` → SQLSTATE `23514`. |
| `mig_0047_listed_in_inventory` | `migration_inventory.toml` contains a `0047_audit_outbox_import_source_devin` row with a non-empty SHA-256. |

## 4. Demo-mode regression tests

| ID | Command | Asserts |
|----|---------|---------|
| `T4.1` | `make -C deploy/demo demo-verify-import-devin-fixture` exits 0 | Verifier SQL asserts ≥ 1 audit row with `import_source = 'devin_team_api'` AND `reservation_source = 'subscription_meter'`; `ledger_entries` rowcount unchanged. |
| `T4.2` | `make -C deploy/demo demo-verify-import-devin-fixture` re-run | Idempotent — same rowcount after second run. |
| `T4.3` | `make -C deploy/demo demo-verify-subscription-meter-claude-code` exits 0 (D13 regression) | D13 path still works; D14 mig 0047 didn't break D13. |
| `T4.4` | `make -C deploy/demo demo-verify-litellm-real` exits 0 (BYOK regression) | BYOK ledger path still works. |

## 5. CloudEvent schema doc golden

`services/importer_devin/tests/cloudevent_envelope_golden.rs`:

| Test | Asserts |
|------|---------|
| `cloudevent_envelope_matches_schema_doc_required_fields` | Parses `docs/specs/coverage/D14_devin_importer/cloudevent-schema.md`, extracts the JSON `Required:` field list, asserts every required field is present in the built envelope. Prevents impl/doc drift. |
| `cloudevent_envelope_v1alpha1_golden` | Built envelope (with frozen UUIDv7 + time) serializes byte-equal to `tests/golden/cloudevent_v1alpha1.json`. Any envelope change requires golden regen + doc update in the same PR. |

## 6. Negative / red-team tests

| Test | Asserts |
|------|---------|
| `fixture_loader_rejects_real_devin_team_id_pattern` | If a fixture file contains a Devin team ID not matching `TEAM_FIXTURE_\d{3}`, loader returns `Err` (prevents accidental real-data commit). |
| `acu_to_micro_usd_overflow_saturates` | `acu = f64::MAX` → result is `i64::MAX` (saturating), no panic. |
| `acu_to_micro_usd_rejects_nan` | `acu = f64::NAN` → `Err(PriceLookupError::InvalidAcuValue)`. |
| `acu_to_micro_usd_rejects_negative` | `acu = -1.0` → `Err(PriceLookupError::InvalidAcuValue)`. |
| `import_record_subject_does_not_leak_token` | `subject` field never contains the Devin API token (live mode does not concat token into subject). |
| `cloudevent_envelope_serialization_no_extra_fields` | Serialized JSON top-level keys are exactly the CloudEvent 1.0 spec-required set + `subject` + `datacontenttype`. No `_internal_*` fields leak. |

## 7. Performance gates

| Test | Asserts |
|------|---------|
| `import_record_to_audit_row_p99_under_50us` | 10k iterations → p99 < 50 µs (pure CPU, no allocation beyond `String::clone`). |
| `cloudevent_envelope_build_p99_under_100us` | 10k iterations → p99 < 100 µs. |
| `fixture_loader_records_p99_under_5ms_for_1k_records` | 1k-record fixture parse + records emission → p99 < 5 ms. |

## 8. Test inventory summary

- Unit tests: ~37 across `acu_price_table.rs`, `import_record.rs`, `cloudevent_envelope.rs`, `fixture_loader.rs`, `live::client` (gated).
- Fixture round-trip: 5.
- Migration: 5.
- Demo regression: 4.
- Schema-doc golden: 2.
- Negative / red-team: 6.
- Performance: 3.

Total ~62 tests. **None require live Devin API access in default CI.** The `live` feature tests use `wiremock`; the optional live-API gate runs only when `DEVIN_API_TOKEN` env var is set.
