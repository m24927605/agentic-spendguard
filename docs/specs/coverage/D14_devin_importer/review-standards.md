# D14 — Review Standards

Slice-specific checklist for the `superpowers:code-reviewer` skill across `COV_67` … `COV_72`. Each slice review consults this file plus [`acceptance.md`](acceptance.md) plus the repo-wide coding standards.

## 1. Threat-model assertions

D14 introduces a new HTTP client (live mode), a CloudEvent emitter that lands rows in `audit_outbox`, a Cargo feature flag boundary that must keep the default build HTTP-free, and a price table that determines dashboard dollar figures. Any diff touching `services/importer_devin/src/{live,acu_price_table,import_record,cloudevent_envelope}.rs`, `tests/fixtures/`, or the migration MUST be reviewed against these assertions; reviewer flags as Blocker if any fails.

| ID | Assertion |
|----|-----------|
| `T1` | `DEVIN_API_TOKEN` value MUST NOT appear in any log, error message, span attribute, or `Display`/`Debug` output. Reviewer greps the diff for `tracing::*!()` macros near the token variable and reads each match. `LiveError::MissingToken` may name the env-var ("DEVIN_API_TOKEN") but never its contents. |
| `T2` | The `live` Cargo feature MUST gate every HTTP-client dependency. Reviewer greps `Cargo.toml` for `reqwest`, `hyper-tls`, `native-tls`, `openssl-sys`, `tokio` (with rt features) and confirms each appears under `optional = true` with the corresponding `live = ["dep:…"]` row. |
| `T3` | The default-feature `cargo tree` output MUST NOT contain any HTTP client crate. Enforced by acceptance `A2.4`. Reviewer confirms the assertion is wired into CI, not just available as a manual check. |
| `T4` | The `live` build MUST use rustls, NEVER native-tls or openssl-sys (matches existing `egress_proxy` rustls policy). Reviewer reads `reqwest` feature list — must include `rustls-tls` and must NOT include `default-tls` or `native-tls`. |
| `T5` | The fixture file `tests/fixtures/devin_usage.json` MUST contain ONLY synthetic Devin team / session IDs matching `^TEAM_FIXTURE_\d{3}$` / `^SESSION_FIXTURE_\d{3}$`. Reviewer greps the fixture for any ID not matching the sentinel pattern — any match is a Blocker. |
| `T6` | `tests/fixtures/PROVENANCE.md` MUST pin the SHA-256 of the fixture generator script. Reviewer recomputes `sha256sum scripts/<generator>.py` and confirms it matches the pinned hex. |
| `T7` | The CloudEvent `subject` field MUST NOT contain the Devin API token, the customer's email, or any inbound credential. The format is `tenant/<tid>/devin/team/<dt>/session/<ds>` — only synthetic identifiers from the import record. |
| `T8` | The `import_record_to_audit_row` function MUST be **pure** — no I/O, no global state, no clock read. Reviewer reads the signature and confirms all dependencies are passed in by reference. Reasoning: makes fuzz / property testing trivial and prevents accidental side channels. |
| `T9` | Live-mode HTTP errors (401/403/429/5xx) MUST surface as typed `LiveError` variants, NEVER as `anyhow::Error` strings that include the response body (could leak vendor PII). Reviewer reads `errors.rs` and confirms each variant is structured. |
| `T10` | The price table asset (`assets/devin_acu_prices.json`) is committed at a stable path. Any change to `usd_per_acu` MUST also bump `pricing_version`. Reviewer verifies a unit test (`price_table_version_bumps_with_rate_change`) asserts this invariant at build time. |
| `T11` | The `live` poll loop MUST have a maximum backoff cap (e.g. ≤ 1 hour). Reviewer reads `poll_loop.rs` and confirms exponential backoff is bounded; an unbounded loop could DoS the Devin API. |
| `T12` | Idempotency key `(devin_team_id, devin_session_id, window_end)` MUST be deterministic — same record produces the same UUIDv7 namespace + name → same `event.id`. Reviewer verifies the impl uses `Uuid::new_v5(namespace, name)` or equivalent stable hashing, NOT random UUIDv7 for re-runnable records. |

## 2. CloudEvent schema assertions

`COV_70` ships the `cloudevent-schema.md` sibling doc + the `cloudevent_envelope.rs` builder. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `S1` | CloudEvent `type` is the **exact** string `spendguard.audit.import.devin_acu`. No variant casing. No version suffix in `type` (version lives in `data.schema_version`). Reviewer greps `cloudevent_envelope.rs` for the constant. |
| `S2` | CloudEvent `source` is the **exact** string `spendguard-importer-devin` — matches the crate name. |
| `S3` | `data.schema_version` is `v1alpha1` for D14. Any future evolution lands as `v1alpha2` (additive) per design §4.3 locked decision. Reviewer confirms the doc states the rule. |
| `S4` | The schema doc (`cloudevent-schema.md`) lists every `data` field with `name`, `type`, `required?`, `description`. Reviewer cross-checks against the `CloudEventData` struct field-by-field. Drift is a Blocker. |
| `S5` | The golden test `cloudevent_envelope_v1alpha1_golden` is byte-equal — any envelope change requires both `cloudevent_envelope.rs` and `tests/golden/cloudevent_v1alpha1.json` and `cloudevent-schema.md` updated in the same PR. Reviewer rejects diffs that modify only one of the three. |
| `S6` | `data.reservation_source` is the **constant** string `subscription_meter` — never `byok`, never `unspecified`. This is what makes the row skip the BYOK ledger in canonical_ingest (D13 §4.3 rule). |
| `S7` | `data.import_source` is the **constant** string `devin_team_api`. Reviewer confirms the value matches the CHECK constraint added by mig 0047. |
| `S8` | `data.ingestion_mode` is exactly one of `"fixture"` or `"live"`. Reviewer confirms the serializer rejects any other value. |
| `S9` | `data.fixture_provenance_sha256` is `Some(<64 hex>)` when `ingestion_mode == "fixture"`, `None` when `ingestion_mode == "live"`. Reviewer confirms a unit test enforces this mode-conditional invariant. |
| `S10` | `data.amount_micro_usd` is **nullable** — enterprise plan emits `null`. The schema doc explicitly documents the nullable case and the matching `reason_code = "devin_enterprise_negotiated_rate"`. |

## 3. ACU → $ conversion correctness

`COV_68` ships the price table loader + conversion. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `C1` | `acu_to_micro_usd` is **pure** and **deterministic** — same `(acu_consumed, usd_per_acu)` always yields the same `i64`. Reviewer confirms no `chrono::Utc::now()` or `rand::*` calls in the conversion path. |
| `C2` | Conversion rounds via `.round()` (round-half-away-from-zero) for cross-language consistency. NOT `.trunc()` (truncates toward zero) or `.floor()`. |
| `C3` | Negative ACU values return `Err`, never produce a negative `amount_micro_usd`. Audit rows with negative spend are semantically invalid. |
| `C4` | `acu = f64::NAN` and `acu = f64::INFINITY` return `Err`. NEVER produce `i64::MIN` / `i64::MAX` rows via cast UB. |
| `C5` | Overflow saturates to `i64::MAX` rather than panics (`as i64` cast UB in debug mode). Reviewer confirms `saturating` arithmetic or explicit overflow check. |
| `C6` | Enterprise plan with `usd_per_acu = None` MUST produce `amount_micro_usd = None` AND `reason_code = Some("devin_enterprise_negotiated_rate")`. Reviewer confirms unit test `import_record_to_audit_row_enterprise_plan_nulls_amount` enforces both. |
| `C7` | `pricing_version` is stamped on every audit row from the **price table at the moment of conversion**, not from a side cache or config. Prevents back-revision of the rate file from retroactively changing historical rows. |

## 4. Crate boundary / feature-flag assertions

`COV_67` + `COV_71` define the feature-flag boundary. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `F1` | `[features] default = []` (empty). The crate ships zero default features. Live mode is opt-in only. |
| `F2` | Every dependency in the `live`-feature deps tree (`reqwest`, `tokio`, `url`) is declared as `optional = true`. Reviewer reads `Cargo.toml`. |
| `F3` | The `src/live/mod.rs` and every file under `src/live/` is `cfg(feature = "live")`-gated at the module declaration in `lib.rs`. No `live`-only symbols leak into the default build. |
| `F4` | The crate is `publish = false` (D13 importer-stub convention). Reviewer reads `Cargo.toml`. |
| `F5` | The crate is registered as a workspace member in the root `Cargo.toml` so workspace-wide commands cover it. |
| `F6` | `live`-feature build has no `panic!`/`unwrap()` on the network path. Reviewer greps `src/live/` and confirms every `Result` is propagated, not unwrapped. |

## 5. Fixture / provenance assertions

`COV_69` ships the fixture and PROVENANCE.md. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `P1` | Fixture file is < 1 MiB. Reviewer runs `du -h services/importer_devin/tests/fixtures/devin_usage.json` and confirms. |
| `P2` | Fixture is valid JSON parseable by both `serde_json::from_str` and `jq` (round-trip canonicalization stable). |
| `P3` | Fixture contains ≥ 1 record per `plan` variant (`team` + `enterprise`) so both code paths are exercised. |
| `P4` | Fixture contains exactly one record per `(devin_team_id, devin_session_id, window_end)` tuple — idempotency-key uniqueness, not multiple rows with the same key. |
| `P5` | `PROVENANCE.md` pins (a) capture date / generator-run date, (b) operator initials, (c) generator script path + SHA-256, (d) "no PII / no real team data" assertion. Reviewer reads each section. |
| `P6` | Fixture `Authorization` header (if any) is `FAKE_DEVIN_TOKEN_*` sentinel only. Reviewer greps the fixture file for any non-sentinel bearer pattern. |

## 6. Migration / inventory assertions

`COV_69` ships migration 0047. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `G1` | Migration number 0047 is contiguous with D13's 0046 and doesn't collide. Reviewer runs `ls services/canonical_ingest/migrations/004*.sql`. |
| `G2` | `migration_inventory.toml` updated with mig 0047 + a non-empty SHA-256 checksum per existing convention. |
| `G3` | Migration is purely additive — `DROP CONSTRAINT IF EXISTS` + `ADD CONSTRAINT` widens the CHECK enum from 2 → 3 values. Reviewer confirms no DROP of `import_source` column and no narrowing of any other constraint. |
| `G4` | Down-migration exists if existing convention requires it (cross-check with the existing 0046 down). |
| `G5` | Mig 0047 leaves D13's partial index `idx_audit_outbox_import_source` intact and operative. Reviewer confirms no `DROP INDEX` in the migration. |

## 7. Demo / Makefile assertions

`COV_72` adds the demo target. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `D1` | New Makefile target `demo-verify-import-devin-fixture` follows the existing `demo-verify-*` naming. |
| `D2` | Verifier SQL uses the same `audit_outbox` columns that existing `verify_step_*.sql` files use — no schema drift. |
| `D3` | Demo harness `import_devin_fixture_demo.sh` does NOT send any HTTP request to a real Devin endpoint — fixture mode is purely local. Reviewer reads the script. |
| `D4` | `make demo-clean` removes D14-specific test rows (audit_outbox rows with `import_source = 'devin_team_api'` from the demo tenant). |
| `D5` | Demo target is idempotent — running twice does not double-emit. Verified by `A6.2`. |

## 8. Docs assertions

`COV_72` adds the Starlight integration page. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `C1` | Page explicitly states "SpendGuard cannot enforce Devin spend — only Cognition can" above the fold. |
| `C2` | Page explains ACU → $ conversion + mentions the $2.25/ACU team plan rate. |
| `C3` | Page documents the enterprise NULL-rate caveat with the `reason_code` value. |
| `C4` | Page cross-links to the strategy memo Archetype IV section. |
| `C5` | Page mentions the importer is idempotent and how to safely re-run. |
| `C6` | Embedded JSON examples (CloudEvent sample, price table) are wrapped in `is:raw` per project Astro convention. |
| `C7` | Page mentions the `DEVIN_API_TOKEN` env var requirement for live mode + how to scope the token (Devin Team Admin API scope only — least privilege). |

## 9. R1-R5 escalation criteria

| Round | Blocker count | Action |
|-------|--------------|--------|
| R1 | 0 → MERGE | none |
| R1 | ≥ 1 → dispatch same implementer with findings | typical 2-4 findings on first review (D14 surface: feature-flag boundary + CloudEvent envelope + price table + migration + docs) |
| R2-R4 | drop to 0 → MERGE | follow normal cadence |
| R5 | ≥ 1 Blocker → Staff+ panel arbitration | panel composition per build plan §1.3 |

**R5 panel summarizer override:** Backend Architect (per design §6 locked decision #6). Rationale: D14 has no DoS surface (importer is one-way pull, no synthetic 4xx returned to clients), no auth parsing on a proxy edge, and no cross-tenant isolation surface (each importer run is single-tenant). The dominant risks are schema correctness + CloudEvent contract stability + crate-boundary hygiene — all Backend Architect framings.

## 10. Per-slice review focus

| Slice | Focus areas |
|-------|-------------|
| `COV_67_d14_devin_crate_scaffold` | §4 (F1-F5), §1 (T2-T4) |
| `COV_68_d14_devin_acu_price_table` | §3 (C1-C7), §1 (T10) |
| `COV_69_d14_devin_fixture_import_path` | §1 (T5-T8, T12), §5 (P1-P6), §6 (G1-G5) |
| `COV_70_d14_devin_cloudevent_schema_doc` | §2 (S1-S10), §1 (T7) |
| `COV_71_d14_devin_live_client_behind_feature` | §1 (T1, T9, T11), §4 (F1-F6) |
| `COV_72_d14_devin_demo_and_docs` | §7 (D1-D5), §8 (C1-C7) |

Each slice's review pass only consults the focus areas listed (plus repo-wide standards); the reviewer is NOT asked to re-check the whole list for every slice.
