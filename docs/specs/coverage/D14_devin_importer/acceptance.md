# D14 — Acceptance Gates

Per build plan §3, every gate listed here must be **100% feasible** at slice-spec time: runnable in the current repo state, no third-party action required, reproducible by the `superpowers:code-reviewer` skill.

D14 is an **architecturally-unreachable deliverable** (Archetype IV — Cognition runs the agent loop, SpendGuard cannot gate). Per build plan §3, feasibility for unreachable deliverables = "billing-importer endpoint is testable against a vendor-staged fixture, even if the live admin API is gated. Acceptance includes 'synthetic audit event emitted' as the primary gate, not 'live import succeeded.'"

## 1. Repository-state gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A1.1` | `services/importer_devin/Cargo.toml` exists; pkg name `spendguard-importer-devin` | `cargo metadata --format-version 1 \| jq -e '.packages[] \| select(.name == "spendguard-importer-devin")'` |
| `A1.2` | `Cargo.toml` declares `publish = false` | `grep -qE '^publish\s*=\s*false' services/importer_devin/Cargo.toml` |
| `A1.3` | `Cargo.toml` declares `live` feature with optional deps | `grep -qE 'live\s*=\s*\["dep:reqwest"' services/importer_devin/Cargo.toml` |
| `A1.4` | `src/import_record.rs` exists with public `import_record_to_audit_row` | `grep -qE 'pub fn import_record_to_audit_row' services/importer_devin/src/import_record.rs` |
| `A1.5` | `src/acu_price_table.rs` exists with `AcuPriceTable` + `lookup` | `grep -qE 'pub struct AcuPriceTable' services/importer_devin/src/acu_price_table.rs && grep -qE 'pub fn lookup' services/importer_devin/src/acu_price_table.rs` |
| `A1.6` | `src/cloudevent_envelope.rs` exists with public `build` | `grep -qE 'pub fn build' services/importer_devin/src/cloudevent_envelope.rs` |
| `A1.7` | `src/fixture_loader.rs` exists with `FixtureLoader::records` | `grep -qE 'pub fn records' services/importer_devin/src/fixture_loader.rs` |
| `A1.8` | `src/live/client.rs` exists, `cfg(feature = "live")`-gated | `grep -qE 'pub struct DevinClient' services/importer_devin/src/live/client.rs && grep -qE 'feature\s*=\s*"live"' services/importer_devin/src/lib.rs` |
| `A1.9` | `assets/devin_acu_prices.json` exists with `pricing_version`, `rates[]` | `jq -e '.pricing_version and (.rates \| length > 0)' services/importer_devin/assets/devin_acu_prices.json` |
| `A1.10` | `tests/fixtures/devin_usage.json` exists | `test -f services/importer_devin/tests/fixtures/devin_usage.json` |
| `A1.11` | `tests/fixtures/PROVENANCE.md` exists with generator script SHA-256 pinned | `grep -qE 'sha256:[0-9a-f]{64}' services/importer_devin/tests/fixtures/PROVENANCE.md` |
| `A1.12` | Migration `0047_audit_outbox_import_source_devin.sql` exists | `test -f services/canonical_ingest/migrations/0047_audit_outbox_import_source_devin.sql` |
| `A1.13` | `docs/specs/coverage/D14_devin_importer/cloudevent-schema.md` exists | `test -f docs/specs/coverage/D14_devin_importer/cloudevent-schema.md` |
| `A1.14` | `docs/site-v2/src/content/docs/integrations/devin-billing-importer.md` exists | `test -f docs/site-v2/src/content/docs/integrations/devin-billing-importer.md` |
| `A1.15` | `README.md` adapter integrations table includes "Devin billing importer" row | `grep -qE 'Devin billing importer' README.md` |
| `A1.16` | Fixture uses synthetic IDs only | `! grep -rE '"devin_team_id":\s*"(?!TEAM_FIXTURE_)' services/importer_devin/tests/fixtures/devin_usage.json` |

## 2. Build gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A2.1` | Workspace builds (default features) | `cargo build --workspace --locked` exits 0 |
| `A2.2` | Importer builds (default features) | `cargo build -p spendguard-importer-devin --locked` exits 0 |
| `A2.3` | Importer builds with `live` feature | `cargo build -p spendguard-importer-devin --features live --locked` exits 0 |
| `A2.4` | Default-feature build pulls NO HTTP client | `cargo tree -p spendguard-importer-devin -e=normal \| grep -E '(reqwest\|hyper-tls\|native-tls\|openssl-sys)'` returns nothing (exit 1) |
| `A2.5` | `live`-feature build uses rustls NOT native-tls | `cargo tree -p spendguard-importer-devin --features live -e=normal \| grep -q 'rustls' && ! (cargo tree -p spendguard-importer-devin --features live -e=normal \| grep -E '(native-tls\|openssl-sys)')` |
| `A2.6` | No new warnings (default features) | `cargo build -p spendguard-importer-devin -- -D warnings` exits 0 |
| `A2.7` | Clippy clean for crate (default + live) | `cargo clippy -p spendguard-importer-devin --all-targets -- -D warnings && cargo clippy -p spendguard-importer-devin --features live --all-targets -- -D warnings` exits 0 |
| `A2.8` | `cargo deny check` passes for the new crate's deps | `cargo deny check` exits 0 |

## 3. Unit-test gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A3.1` | `acu_price_table` unit tests green | `cargo test -p spendguard-importer-devin --lib acu_price_table` exits 0 |
| `A3.2` | `import_record` unit tests green | `cargo test -p spendguard-importer-devin --lib import_record` exits 0 |
| `A3.3` | `cloudevent_envelope` unit tests green | `cargo test -p spendguard-importer-devin --lib cloudevent_envelope` exits 0 |
| `A3.4` | `fixture_loader` unit tests green | `cargo test -p spendguard-importer-devin --lib fixture_loader` exits 0 |
| `A3.5` | Live-client `wiremock` tests green (gated) | `cargo test -p spendguard-importer-devin --features live --lib live::client` exits 0 |
| `A3.6` | Contract tests green | `cargo test -p spendguard-importer-devin import_record_to_audit_row_sets_subscription_meter import_record_to_audit_row_sets_import_source_devin_team_api` exits 0 |
| `A3.7` | Enterprise-NULL-rate test green | `cargo test -p spendguard-importer-devin import_record_to_audit_row_enterprise_plan_nulls_amount` exits 0 |

## 4. Fixture round-trip integration-test gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A4.1` | Fixture round-trip tests green | `cargo test -p spendguard-importer-devin --test fixture_round_trip` exits 0 |
| `A4.2` | Idempotency test green | `cargo test -p spendguard-importer-devin --test fixture_round_trip fixture_round_trip_is_idempotent` exits 0 |
| `A4.3` | Enterprise-NULL fixture round-trip green | `cargo test -p spendguard-importer-devin --test fixture_round_trip fixture_enterprise_record_lands_with_null_amount` exits 0 |
| `A4.4` | Pricing-version stamping test green | `cargo test -p spendguard-importer-devin --test fixture_round_trip fixture_pricing_version_stamped` exits 0 |

## 5. Schema migration gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A5.1` | Migration applies cleanly | `psql "$DATABASE_URL" -f services/canonical_ingest/migrations/0047_audit_outbox_import_source_devin.sql` exits 0 |
| `A5.2` | Migration idempotent | apply twice → second run still exits 0 |
| `A5.3` | CHECK accepts `devin_team_api` | `psql -c "INSERT INTO audit_outbox (… , import_source) VALUES (…, 'devin_team_api');"` succeeds |
| `A5.4` | CHECK still accepts D13 values | INSERT with `import_source = 'anthropic_console_usage'` or `'openai_admin_usage'` succeeds |
| `A5.5` | CHECK rejects unknown value | INSERT with `import_source = 'wat'` returns SQLSTATE `23514` |
| `A5.6` | Migration listed in `migration_inventory.toml` with checksum | `grep -qE '0047_audit_outbox_import_source_devin' services/canonical_ingest/migration_inventory.toml` |

## 6. Demo-mode regression gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A6.1` | `make -C deploy/demo demo-verify-import-devin-fixture` exits 0 | Replays fixture → verifier SQL asserts ≥ 1 row with `import_source = 'devin_team_api'` AND `reservation_source = 'subscription_meter'`; `ledger_entries` unchanged. |
| `A6.2` | Demo target idempotent | Run twice → both succeed; PG rowcount unchanged after second run. |
| `A6.3` | Verifier SQL committed | `test -f deploy/demo/verify_step_import_devin_fixture.sql` |
| `A6.4` | Pre-existing D13 demo regression: `make -C deploy/demo demo-verify-subscription-meter-claude-code` still exits 0 | D14 mig 0047 doesn't break D13. |
| `A6.5` | Pre-existing BYOK demo regression: `make -C deploy/demo demo-verify-litellm-real` still exits 0 | Ledger path intact. |
| `A6.6` | `make demo-clean` removes D14-specific artefacts | After clean, audit_outbox rows with `import_source = 'devin_team_api'` purged. |

## 7. CloudEvent schema gates

Per design §4.3, the `spendguard.audit.import.devin_acu` CloudEvent schema is a deliverable artefact. The schema doc is committed at `docs/specs/coverage/D14_devin_importer/cloudevent-schema.md` and the impl is golden-tested against it.

| ID | Gate | Verification command |
|----|------|----------------------|
| `A7.1` | Schema doc declares all `data` fields from design §4.3 | `for f in schema_version tenant_id budget_id devin_team_id devin_session_id acu_consumed usd_per_acu amount_micro_usd pricing_version window_start window_end reservation_source import_source ingestion_mode fixture_provenance_sha256; do grep -qE "\`$f\`" docs/specs/coverage/D14_devin_importer/cloudevent-schema.md \|\| exit 1; done` |
| `A7.2` | Schema doc pins `type` constant | `grep -qE 'spendguard\.audit\.import\.devin_acu' docs/specs/coverage/D14_devin_importer/cloudevent-schema.md` |
| `A7.3` | Schema doc declares `schema_version = "v1alpha1"` | `grep -qE 'v1alpha1' docs/specs/coverage/D14_devin_importer/cloudevent-schema.md` |
| `A7.4` | Schema golden test green | `cargo test -p spendguard-importer-devin --test cloudevent_envelope_golden cloudevent_envelope_v1alpha1_golden` exits 0 |
| `A7.5` | Schema-doc-required-fields test green | `cargo test -p spendguard-importer-devin --test cloudevent_envelope_golden cloudevent_envelope_matches_schema_doc_required_fields` exits 0 |
| `A7.6` | Schema doc explicitly states "additive evolution → v1alpha2" rule | `grep -qE 'additive\|v1alpha2' docs/specs/coverage/D14_devin_importer/cloudevent-schema.md` |

## 8. Performance gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A8.1` | `import_record_to_audit_row` p99 < 50 µs | `cargo test -p spendguard-importer-devin --release -- --ignored import_record_to_audit_row_p99_under_50us` exits 0 |
| `A8.2` | `cloudevent_envelope::build` p99 < 100 µs | `cargo test -p spendguard-importer-devin --release -- --ignored cloudevent_envelope_build_p99_under_100us` exits 0 |
| `A8.3` | `FixtureLoader::records` p99 < 5 ms for 1k records | `cargo test -p spendguard-importer-devin --release -- --ignored fixture_loader_records_p99_under_5ms_for_1k_records` exits 0 |

## 9. Security gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A9.1` | Live client never logs the bearer token | `cargo test -p spendguard-importer-devin --features live live_client_does_not_log_token` exits 0 |
| `A9.2` | `subject` field never contains the bearer token | `cargo test -p spendguard-importer-devin import_record_subject_does_not_leak_token` exits 0 |
| `A9.3` | Live client uses rustls, never native-tls | covered by `A2.5` |
| `A9.4` | Fixture has no real Devin team IDs | `A1.16` regex enforcement |
| `A9.5` | PROVENANCE.md pins generator script SHA-256 | `A1.11` |
| `A9.6` | Live client `MissingToken` error never leaks the env-var contents (only the env-var name) | `cargo test -p spendguard-importer-devin --features live live_client_missing_token_errors_clearly` reads the `Display` output and confirms no `Bearer` substring. |

## 10. Acceptance scenario gate (primary headline gate)

**The headline acceptance scenario** (from the deliverable prompt):

> Without live API access: fixture-driven test imports a recorded `devin_usage.json` snapshot and emits synthetic audit events with correct ACU → $ conversion. Audit event shape matches the `spendguard.audit.import.devin_acu` CloudEvent schema.

This is verified by `A10.1` — `A10.3`:

| ID | Gate | Verification command |
|----|------|----------------------|
| `A10.1` | Fixture replay emits ≥ 1 `import_source = 'devin_team_api'` audit row | `make -C deploy/demo demo-verify-import-devin-fixture` exits 0; verifier SQL asserts row count, `amount_micro_usd > 0` for `team` plan, `amount_micro_usd IS NULL` for `enterprise` plan with `reason_code = 'devin_enterprise_negotiated_rate'`. |
| `A10.2` | Audit event shape matches schema doc | `A7.4` + `A7.5` green. |
| `A10.3` | ACU → $ conversion correct (12.5 ACU × $2.25/ACU → 28,125,000 micro-USD) | `cargo test -p spendguard-importer-devin import_record_to_audit_row_amount_conversion` exits 0. |

`A10.1` is the **merge-blocking gate** that maps directly to the deliverable prompt's primary acceptance criterion. `A10.2` enforces schema fidelity; `A10.3` enforces conversion correctness.

## 11. Live API gate (optional, env-var-gated)

Per design §3 + build plan §3, **the live API gate is NOT a merge gate**. It runs only when `DEVIN_API_TOKEN` is present (developer local validation or vendor-staged CI).

| ID | Gate | Verification command |
|----|------|----------------------|
| `A11.1` | Live `from_env` constructs successfully when token present | `DEVIN_API_TOKEN=test cargo test -p spendguard-importer-devin --features live live_client_constructs_from_env` exits 0 |
| `A11.2` | Live 24h pull (developer-gated) | `DEVIN_API_TOKEN=<real> DEVIN_TEAM_ID=<real> cargo run -p spendguard-importer-devin --features live --bin importer_devin -- --mode live --since 24h --dry-run` exits 0 and prints ≥ 1 record. Not run in CI by default. |

`A11.x` are **observability gates**, not merge gates. CI absence of `DEVIN_API_TOKEN` skips them silently.

## 12. Documentation gates

| ID | Gate | Verification command |
|----|------|----------------------|
| `A12.1` | Doc page explicitly says "reconciliation only, no gating" | `grep -qE 'reconciliation.*only\|cannot.*gate\|cannot.*enforce' docs/site-v2/src/content/docs/integrations/devin-billing-importer.md` |
| `A12.2` | Doc page explains ACU → $ conversion + price table | `grep -qE 'ACU\|Agent Compute Unit' docs/site-v2/src/content/docs/integrations/devin-billing-importer.md` |
| `A12.3` | Doc page mentions enterprise NULL-rate caveat | `grep -qE 'enterprise.*negotiated\|amount_micro_usd.*null\|reason_code' docs/site-v2/src/content/docs/integrations/devin-billing-importer.md` |
| `A12.4` | Doc page cross-links to strategy memo Archetype IV | `grep -qE 'framework-coverage-2026-06\|Archetype IV' docs/site-v2/src/content/docs/integrations/devin-billing-importer.md` |
| `A12.5` | Embedded JSON example wrapped in `is:raw` (Astro convention) | `grep -qE 'is:raw' docs/site-v2/src/content/docs/integrations/devin-billing-importer.md` |
| `A12.6` | README adapter table row present | `A1.15` |

## 13. Anti-regression gate

| ID | Gate | Verification command |
|----|------|----------------------|
| `A13.1` | D13 importer stub still builds | `cargo build -p spendguard-importer-anthropic -p spendguard-importer-openai --locked` exits 0 |
| `A13.2` | D13 mig 0046 backwards compat | Existing rows with `import_source IN ('anthropic_console_usage', 'openai_admin_usage')` still pass CHECK after mig 0047 applies. |
| `A13.3` | Existing canonical_ingest CloudEvent parse path still works | `cargo test -p spendguard-canonical-ingest --test cloudevent_parse_smoke` exits 0 |
| `A13.4` | No existing audit_outbox row's behaviour changes | Pre-/post-D14 query: `SELECT count(*) FROM audit_outbox WHERE import_source IS NULL` is monotonically non-decreasing across the migration. |

`A13.x` collectively ensures D14 is purely additive — no D13 or BYOK regression.
