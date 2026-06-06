# D15 — Review Standards

Slice-specific checklist for the `superpowers:code-reviewer` skill across `COV_70` … `COV_74`. Each slice review consults this file plus [`acceptance.md`](acceptance.md) plus repo-wide coding standards.

## 1. Threat-model assertions

D15 introduces a Rust crate that reads vendor admin REST credentials, parses JSON over HTTPS (in `live` mode), converts credits to dollars via a pricing table, and writes synthetic audit rows tagged `reservation_source = 'import_manus'`. The surface is narrower than D13 (no inbound traffic, no synthetic 429), so the security weight shifts from "Authorization parsing hot path" to "vendor-token handling + fail-closed pricing." Reviewer flags as Blocker if any of the following fails:

| ID | Assertion |
|----|-----------|
| `T1` | `MANUS_API_TOKEN` is **never** logged. Reviewer greps the diff for `tracing::*!()` calls that touch the `token` field, `bearer_auth(`, or `Authorization` header values, and confirms each call site logs at most metadata (HTTP status, retry count) — not the token. |
| `T2` | `MANUS_API_TOKEN` is **never** committed in fixtures. Reviewer greps `services/importer_manus/tests/fixtures/` for any string matching `^[A-Za-z0-9_-]{32,}$` that lacks the `FAKE_` sentinel prefix. |
| `T3` | The `live` cargo feature is **opt-in**. Reviewer reads `Cargo.toml` and confirms `default = []` and that all `reqwest` / network-stack deps are marked `optional = true`. |
| `T4` | `cargo tree -p spendguard-importer-manus -e=normal` shows zero HTTP-client deps when the default features build. Manually verified at slice time. |
| `T5` | `live::from_env` fails closed on missing or empty `MANUS_API_TOKEN` — no fallback URL, no anonymous request, no panic. |
| `T6` | Unknown tier in a fixture or live response = **skip + WARN**, never invent a USD amount. Reviewer reads the matching test (`load_fixture_unknown_tier_returns_err` / `live_poll_usage_skips_malformed_records_with_warn`) and confirms the assertion. |
| `T7` | Synthetic audit rows are written via the canonical `append_audit_outbox()` API with parameterised SQL — no string-formatted queries with vendor-controlled fields (`session_id`, `workspace_id`). |
| `T8` | Workspace IDs and session IDs in the committed fixture are sentinel-prefixed (`ws_FAKE_…`, `ses_FAKE_…`); no real customer identifiers leak in. Cross-checked against `PROVENANCE.md`. |
| `T9` | `PROVENANCE.md` pins the SHA-256 of the redaction script. Reviewer recomputes `sha256sum scripts/redact_har.py` (or equivalent JSON redactor) and confirms the match. |
| `T10` | The importer NEVER writes to `ledger_entries` or `reservations` — verified by `fixture_import_no_ledger_write` test. Mirrors D13 §1 T10 invariant. |
| `T11` | `live` HTTP client uses `rustls-tls`, NOT `native-tls`. Project policy bans `native-tls` to avoid OS root-store ambiguity. |
| `T12` | `live` HTTP client has a request timeout configured (≤ 30s default). Open-ended polling against an unresponsive vendor is a denial-of-availability risk to the importer scheduler. |
| `T13` | Saturating arithmetic in `credit_to_usd_micros`. Reviewer reads the function and confirms `saturating_mul` is used; a `*` operator on i64 is a Blocker (overflow panics in release on debug-builds; silent wrap in release builds — both bad). |
| `T14` | The `dedupe_key` includes the Manus `session_id` — ensures double-import is idempotent at the canonical_ingest layer, not just at the importer layer. |

## 2. Pricing-table integrity assertions

`COV_71` ships the price table. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `P1` | `team_plan.credit_cost_micro_usd = 20_526` exactly (39.00 / 1900 × 1_000_000, integer-rounded). Reviewer recomputes the math: `(39.00 / 1900.0) * 1_000_000.0 ≈ 20526.3`; the integer ROUND is `20_526` (truncating toward 0). Documented in a TOML comment so future operators understand the truncation. |
| `P2` | `enterprise.credit_cost_micro_usd = 0` (default). Operators are expected to override at deploy time. Documented in the integration page. |
| `P3` | `enterprise_byok.credit_cost_micro_usd = 0` is **load-bearing** — BYOK tier customers pay the LLM provider directly, so the importer must NOT double-bill them. Reviewer reads the comment in the TOML asserting this rationale. |
| `P4` | No floating-point math in the conversion hot path. `f64` only appears in the price-table parse step (TOML deserialisation) — once converted to `i64 micro_usd`, all downstream math is integer. |
| `P5` | Price table loader uses `include_str!` so the table is embedded in the binary; no runtime file IO required for the default fixture path. Live mode does not change this — overrides come from env or k8s ConfigMap, not the embedded asset. |

## 3. Migration assertions

`COV_74` ships migrations 0047 + 0048. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `G1` | Migration numbers are contiguous and don't collide with D13's 0044/0045/0046. `ls services/canonical_ingest/migrations/004[7-8]*.sql` returns exactly the two D15 files. |
| `G2` | Migration 0047 enumerates **all five** values in the CHECK: `'byok'`, `'subscription_meter'`, `'import_devin'`, `'import_manus'`, `'import_genspark'`. Forward-compat slots for D14 and D16 prevent a follow-up CHECK-rewrite migration. |
| `G3` | Migration 0048 enumerates **all five** `import_source` values: `'anthropic_console_usage'`, `'openai_admin_usage'`, `'devin_admin_usage'`, `'manus_admin_usage'`, `'genspark_admin_usage'`. Same forward-compat principle. |
| `G4` | Down-migrations restore the D13 narrower CHECK exactly — not a rougher "drop and re-add a more permissive CHECK." Reviewer reads both down files and confirms. |
| `G5` | Migrations are additive: pre-existing rows with `reservation_source = 'byok'` or `'subscription_meter'` remain queryable after applying 0047. No `UPDATE` statement in either migration. |
| `G6` | Partial index `idx_audit_outbox_import_manus` predicate matches the new CHECK value exactly: `WHERE reservation_source = 'import_manus'` — no typos, no broader filter. |
| `G7` | `migration_inventory.toml` SHA-256 pins are recomputed and the file is checked in with the correct hashes. Reviewer runs `sha256sum services/canonical_ingest/migrations/0047_*.sql services/canonical_ingest/migrations/0048_*.sql` and compares. |
| `G8` | Migrations run cleanly against the demo Postgres that already has D13's 0044/0045/0046 applied. Tested via `make -C deploy/demo demo-up`. |

## 4. ETL correctness assertions

`COV_72` ships the fixture import path. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `E1` | `load_fixture` validates **before** producing `ImportRecord`. Tier and status are checked at parse time, not at insert time. A bad fixture fails fast with a clear error, not partway through a stream of inserts. |
| `E2` | The pipeline is `load_fixture → import_record_to_audit_row → append_audit_outbox` — three pure steps, two of which compose without IO. The IO (DB insert) is one function, easy to mock. Reviewer rejects diffs that interleave the steps. |
| `E3` | `in_progress` sessions are **loaded** but **not emitted as audit rows** in the demo path. The demo runtime script applies the policy (not the loader); the loader stays general so the live mode can include `in_progress` in a "what's in flight" dashboard variant if/when that's wanted. Reviewer confirms the demo's filter step. |
| `E4` | `model` field on every emitted row is the **fixed synthetic identifier** `"manus.session/credit"`. Reviewer rejects diffs that try to fish an LLM model name out of session metadata — Manus does not expose it, and a per-row guess corrupts the analytics. |
| `E5` | `input_tokens = 0` and `output_tokens = 0` on every emitted row. Token-level attribution is not available; reporting zero is honest; reporting an estimate would mislead downstream Strategy A predictions. |
| `E6` | `dedupe_key = Some(format!("manus:{session_id}"))` — vendor-prefixed so D14 / D16 keys don't collide with D15. Reviewer reads the format string and confirms the prefix. |
| `E7` | `occurred_at = window_end` — matches D13 §5 Anthropic-importer convention. Consistency lets dashboards apply a single time-window filter across all importers. |
| `E8` | The fixture-driven test is **deterministic** (no clocks, no random IDs). Two consecutive runs of `cargo test fixture_import_emits_seven_audit_events` produce byte-identical DB state. |

## 5. Live-mode (`COV_73`) assertions

`COV_73` ships the HTTP client behind `--features live`. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `L1` | `LiveClient::from_env` is the **only** public constructor — no `LiveClient::new(token: String)` that would invite hard-coded tokens in dev code. Reviewer rejects a public new-from-token constructor unless documented as `#[cfg(test)]`. |
| `L2` | Pagination loop has a **hard upper bound** (e.g. 10_000 pages). A vendor-misbehaving `next_cursor` that loops forever must terminate. Reviewer reads the loop and confirms the bound. |
| `L3` | Cursor parameter is sent only when non-empty. A misread `"next_cursor": ""` does NOT add an empty cursor to the next request. |
| `L4` | Errors from `reqwest` are wrapped via `thiserror`-derived `ImporterError`; never `unwrap` or `expect`. Reviewer greps the diff for `.unwrap()` / `.expect(` and rejects any in the `live` module. |
| `L5` | The mock-server test (`live_poll_usage_with_token_succeeds`) hits a `httpmock`-bound localhost URL — no test issues a request to `api.manus.ai` even with `MANUS_API_TOKEN` unset. |
| `L6` | The `live` module is **only** wired into `lib.rs` via `#[cfg(feature = "live")]`, not via runtime config. A non-`live` build cannot accidentally call into the HTTP path. |
| `L7` | User-Agent string is `spendguard-importer-manus/<version>` exactly — lets Manus identify SpendGuard traffic in their server logs (good citizen + lets them rate-limit us specifically if they choose). |
| `L8` | Token is moved into `LiveClient` (not borrowed) — prevents lifetime-aliasing accidents where the token outlives the client and lands in a tracing event. |

## 6. CloudEvent / outbox-forwarder assertions

`COV_74` registers the new CloudEvent type. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `C1` | The constant `CLOUDEVENT_TYPE = "spendguard.audit.import.manus_credit"` is the SINGLE source of truth — referenced from `audit.rs`, the outbox forwarder registry, the verifier SQL, and the integration doc. Reviewer greps for the literal string and confirms no drift. |
| `C2` | The CloudEvent type fits the family pattern `spendguard.audit.import.<vendor>_<unit>` — siblings D14 will use `…devin_acu`, D16 will use `…genspark_acu`. Reviewer cross-references the design memo. |
| `C3` | Outbox forwarder routes the new type through the same downstream sinks as the existing audit types (no new sink configured by D15). Reviewer reads the sink config and confirms inheritance. |
| `C4` | Schema of the CloudEvent payload reuses the existing `AuditRow` shape — no parallel schema for Manus rows. Reviewer rejects diffs introducing a `ManusAuditRow` parallel struct. |

## 7. Demo / Makefile assertions

`COV_74` adds the demo target. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `D1` | New Makefile target follows the existing `demo-*` naming convention (`demo-import-manus-fixture`). |
| `D2` | Verifier SQL `verify_step_import_manus.sql` uses parameterised `assert` calls; numeric expectations (7 rows, 20_731_260 micro-USD) are inline constants commented with the math. |
| `D3` | Demo script does NOT require `MANUS_API_TOKEN` — fixture mode only. The integration page documents how to wire live mode separately. |
| `D4` | Demo target is idempotent. Running it twice produces 7 rows, not 14, because of `dedupe_key`. |
| `D5` | `make demo-clean` removes `import_manus` audit rows. Verifier query `SELECT count(*) FROM audit_outbox WHERE reservation_source = 'import_manus'` returns 0 after clean. |

## 8. Docs assertions

`COV_74` adds the Starlight docs page. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `K1` | The page states **above the fold** that SpendGuard cannot enforce Manus spend — only post-hoc reconciliation. |
| `K2` | The page explains fixture mode AND live mode (with `MANUS_API_TOKEN` setup). |
| `K3` | The page tabulates the three tiers (`team_plan`, `enterprise`, `enterprise_byok`) with their `credit_cost_micro_usd` values and explains the rationale (BYOK = $0 because customer pays the LLM direct). |
| `K4` | The page cross-links to the strategy memo's Archetype IV section. |
| `K5` | The page cross-links to D14 (Devin) and D16 (Genspark) when those specs exist — for now, a note that they are sibling workstreams. |
| `K6` | Embedded JSON examples are wrapped in `is:raw` per project memory (`feedback_astro_is_raw`). |
| `K7` | The page does NOT promise live mode as GA-ready — describes it as `--features live` opt-in for operators with Team+ tier API access. |

## 9. R1-R5 escalation criteria

| Round | Blocker count | Action |
|-------|--------------|--------|
| R1 | 0 → MERGE | none |
| R1 | ≥ 1 → dispatch same implementer with findings | typical 1-3 findings on first review (D15 surface is narrower than D13: no inbound proxy, no synthetic 429, no `tenant_id` resolution to validate) |
| R2-R4 | drop to 0 → MERGE | follow normal cadence |
| R5 | ≥ 1 Blocker → Staff+ panel arbitration | panel composition per build plan §1.3 |

**R5 panel summarizer override:** Backend Architect (per design §5 locked decision #6). Rationale: D15 is schema + crate plumbing + ETL with a narrow security surface (env var + HTTPS client + sentinel-only fixtures). Backend framing dominates over the security framing that ruled D13. Security Engineer remains on the panel and explicitly reviews §1 threat assertions.

## 10. Per-slice review focus

| Slice | Focus areas |
|-------|-------------|
| `COV_70_d15_crate_scaffold` | §1 (T3, T4), §3 (G1) |
| `COV_71_d15_credit_price_table` | §1 (T6, T13), §2 (P1-P5) |
| `COV_72_d15_fixture_import_path` | §1 (T2, T6-T10, T14), §4 (E1-E8), §8 (T8-T9 fixture provenance) |
| `COV_73_d15_live_http_client` | §1 (T1, T5, T11-T12), §5 (L1-L8) |
| `COV_74_d15_audit_emission_and_demo` | §3 (G2-G8), §6 (C1-C4), §7 (D1-D5), §8 (K1-K7) |

Each slice's review pass only consults the focus areas listed (plus repo-wide standards); the reviewer is NOT asked to re-check the whole list for every slice.

## 11. Cross-deliverable consistency (D13 / D14 / D16 family)

D15 is the second importer-family spec written (D13 is the originator of the `import_source` column; D14 and D16 are upcoming siblings). Reviewer specifically guards against:

| ID | Assertion |
|----|-----------|
| `X1` | CHECK enumerations in mig 0047 + 0048 list `'import_devin'` and `'import_genspark'` as forward-compat slots — D14 and D16 must NOT need a follow-up CHECK migration to add their reservation_source values. |
| `X2` | The CloudEvent type follows the `spendguard.audit.import.<vendor>_<unit>` family pattern — `manus_credit`, not `manus`, not `manus_session_credit`, not `manus.credit`. |
| `X3` | `dedupe_key` prefix is `manus:` — D14 will use `devin:`, D16 will use `genspark:`. No collisions across vendors when a customer uses two importers concurrently. |
| `X4` | The `model` field literal `"manus.session/credit"` follows the convention `<vendor>.<unit-grain>/<unit>` — D14 will use `devin.task/acu`, D16 will use `genspark.task/credit`. Reviewer cross-references the family. |
| `X5` | Importer crate structure (`record.rs`, `pricing.rs`, `fixture.rs`, `audit.rs`, `error.rs`, optional `live.rs`) is the **template** for D14 and D16. Reviewer rejects unnecessary divergence (e.g. a `manus_specific_module.rs` named after the vendor — keep the file names vendor-neutral so the template ports cleanly). |
