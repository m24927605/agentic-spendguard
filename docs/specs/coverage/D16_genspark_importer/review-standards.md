# D16 — Review Standards

Slice-specific checklist for the `superpowers:code-reviewer` skill across `COV_84` … `COV_88`. Each slice review consults this file plus [`acceptance.md`](acceptance.md) plus repo-wide coding standards. Mirrors the structure of [`D13`](../D13_subscription_meter/review-standards.md) but adapted to the importer's much narrower threat surface.

## 1. Threat-model assertions

D16 introduces a periodic worker that (a) parses vendor-supplied JSON, (b) reads an environment-variable secret behind the `live` feature, (c) writes synthetic audit rows that **must not** touch the BYOK ledger, and (d) extends D13's `audit_outbox` CHECK constraints. The threat surface is narrower than D13 (no inbound Authorization parsing at the proxy edge, no synthetic 429 DoS, no in-band classifier mistake leading to lost dollars) but a misconfigured importer can still write phantom dollars into operator dashboards or leak the admin-API token.

Any diff touching `services/importer_genspark/src/{audit,emit,live,price,record}.rs`, the `0053` migration, the committed price table, or the fixture set MUST be reviewed against these assertions; reviewer flags as Blocker if any fails.

| ID | Assertion |
|----|-----------|
| `T1` | `GENSPARK_API_TOKEN` is read once at startup and stored in `secrecy::SecretString`. No code path serialises, `Debug`-prints, `Display`-prints, or otherwise embeds the raw token in a string. Reviewer greps the `live` module for `format!.*token`, `to_string`, `{:?}` on the token field, and rejects on hit. |
| `T2` | The runtime gate rejects (a) absent env var, (b) empty-after-trim value, (c) value < 32 chars. Reviewer reads the constructor and confirms all three branches return distinct `Err` values (so operators can debug). |
| `T3` | The compile-time `live` Cargo feature pulls `reqwest` + `secrecy` **only**. Default-features build has neither in the `cargo tree -e=normal` output. Reviewer cross-checks `[features] live = ["dep:reqwest", "dep:secrecy"]` and confirms each dep is `optional = true` in `[dependencies]`. |
| `T4` | `reqwest` is configured with `rustls-tls`, NOT `native-tls`. Reviewer greps `Cargo.toml` for `features = ["rustls-tls"` on the reqwest dep line. |
| `T5` | The importer **never** writes to `ledger_entries` or `reservations`. Reviewer greps the entire crate for `INSERT INTO ledger_entries`, `INSERT INTO reservations`, `sqlx::query.*ledger`, and `sqlx::query.*reservation` — any match in non-test code is a Blocker. (Same invariant as D13 §4.3.) |
| `T6` | `import_record_to_audit_row` is **pure**: signature `fn import_record_to_audit_row(rec: &…, price: &…) -> AuditRow`, all args borrowed, no `&mut`, no IO, no globals. Reviewer reads the signature and the body. |
| `T7` | Unknown plan tier (`credits_to_micro_usd` returns `None`) → `amount_micro_usd = 0` AND `reason_code = Some("genspark_plan_unknown")`. Reviewer confirms BOTH fields are set, not just one. (Setting only `amount_micro_usd = 0` would silently mis-price; setting only `reason_code` would still leave a non-zero $ on the dashboard.) |
| `T8` | The vendor JSON parser uses `#[serde(rename_all = "snake_case")]` and tolerates unknown fields (does not use `#[serde(deny_unknown_fields)]`). Rationale: protects against Genspark's API schema additions causing CI failures across operator deployments. Reviewer confirms `deny_unknown_fields` is NOT present. |
| `T9` | Fixture files contain only `FAKE_ws_*` workspace IDs. No real Genspark workspace ID present. Reviewer greps committed `.json` files for `ws_[A-Za-z0-9_-]{16,}` and confirms every match starts with `FAKE_`. |
| `T10` | Fixture files contain no prompt content. Reviewer greps committed `.json` files for `"content":` and confirms no matches (admin API does not return prompts; any match indicates fixture contamination). |
| `T11` | Migration 0053 is **additive** — the CHECK drop-and-recreate must include ALL D13 values (`'byok'`, `'subscription_meter'`) in the recreated set. Reviewer reads the SQL and confirms. Narrowing the constraint silently would reject D13 inserts post-migration. |
| `T12` | Migration 0053 down-script is documented to FAIL when `'import_genspark'` rows exist — this is intentional, not a bug. Reviewer confirms a code comment in the down-script explains the operator obligation. |
| `T13` | The `live` mode binary path is unreachable on a default build. Reviewer reads `main.rs` and confirms the `--workspace` arg returns a clear error (NOT a panic) when `live` is OFF. |
| `T14` | `bin/main.rs` exits cleanly on a fixture-mode invocation; never prints stack traces or panic backtraces to stderr on the happy path. Reviewer checks `panic!`/`unwrap`/`expect` usage in `main.rs` is bounded to startup-time invariants. |
| `T15` | Negative `credits_consumed` from the API is treated as an arithmetic edge case (saturating multiplication, no panic). Reviewer reads the conversion code; if it uses non-saturating `*`, that's a Blocker. |

## 2. Cross-tier correctness assertions

`COV_85` (price table + record→row contract) and `COV_86` (fixture import path + migration 0053 + audit_outbox writes) cross multiple subsystems. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `X1` | `reservation_source = 'import_genspark'` is a **new distinct value**, not a sub-mode of D13's `'subscription_meter'`. Rationale: cost basis differs (vendor credit aggregate vs. SpendGuard tokenizer estimate); fusing them on the dashboard misleads the CFO. Reviewer reads the audit row construction and confirms the string literal. |
| `X2` | `import_source = 'genspark_billing'` is a **new distinct value** under D13/0046's column. Reviewer confirms the CHECK constraint in 0053 includes both D13 values AND the new value. |
| `X3` | CloudEvent type string is **exactly** `"spendguard.audit.import.genspark_credit"` (matches design §6 decision #5; sibling D14 will use `…import.devin_acu`, D15 `…import.manus_credit`). Reviewer confirms the constant. |
| `X4` | `row.model = format!("genspark/{}", rec.plan)`. The Genspark API hides the upstream model — operators see a synthetic `genspark/plus`-style model name, NOT a fabricated `gpt-4`-style label that would falsely imply we know the upstream provider. Reviewer reads the format string. |
| `X5` | `row.occurred_at = rec.window_end`, NOT `rec.window_start` or `Utc::now()`. Rationale: aligned with D13 §4.3 "as of end of window" semantic; cross-importer consistency. |
| `X6` | `row.pricing_version` comes from the `GensparkPriceTable.pricing_version` field, NOT a hardcoded constant. Audit rows must be queryable by pricing-version for back-dated repricing. |
| `X7` | The committed `genspark_credit_price.toml` has `pricing_version = "genspark-2026-06-06"` — the date matches the strategy memo's capture date. Changing the table requires bumping the version string; reviewer rejects diffs that change pricing without bumping the version. |
| `X8` | Migration 0053 ordering: contiguous with D13's reserved 0044-0046. Reviewer runs `ls services/canonical_ingest/migrations/0053*.sql` and confirms only D16 owns 0053. If D14 (Devin) or D15 (Manus) reserves intermediate numbers, reviewer cross-checks no collision. |
| `X9` | The fixture JSON shape **matches** what the actual Genspark admin API returns. Reviewer cross-references `services/importer_genspark/tests/fixtures/PROVENANCE.md` and confirms the JSON shape is documented (synthetic-but-spec-accurate, NOT invented). |
| `X10` | The `chrono::Utc` clock is used for window math, NOT `chrono::Local`. Reviewer greps the diff for `chrono::Local` and rejects on hit (consistent with D13's `X8`). |

## 3. Price-table correctness matrix

`COV_85` modifies `price.rs`. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `M1` | The price table loader handles `monthly_credits = 0` without dividing by zero. Returns `None` for that plan's `effective_usd_per_credit`. |
| `M2` | The loader handles missing `pricing_version` as an error (not silently defaulting). |
| `M3` | The loader caches `effective_usd_per_credit` once at load time; subsequent `credits_to_micro_usd` calls do NOT redo the division. (Performance + determinism.) |
| `M4` | The loader supports `GENSPARK_PRICE_TABLE_PATH` env override per design §3.1. Reviewer confirms env-var lookup happens at load time, not at every credit-conversion call. |
| `M5` | `credits_to_micro_usd` returns `Option<i64>`, NOT `i64`. Distinguishing unknown-plan (`None`) from zero-credits (`Some(0)`) is the core dashboard correctness invariant. Reviewer rejects any signature that conflates the two. |
| `M6` | Micro-USD conversion uses `(usd * 1_000_000.0) as i64`. Reviewer reads the conversion and confirms no `f64`-to-`i32` cast, no `unwrap` on overflow. |
| `M7` | The committed price table is `.toml`, NOT `.yaml` or `.json`. Consistent with rest-of-repo convention; mechanical parsing. |

## 4. Importer-vs-ledger fork assertions

`COV_86` introduces the audit-row-write path. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `F1` | The write happens via **one** entry point: `emit::write_audit_row(pool, row)`. Reviewer rejects diffs that introduce alternative write paths (e.g. direct `sqlx::query!("INSERT INTO audit_outbox …")` calls scattered through the codebase). |
| `F2` | `emit::write_audit_row` does NOT call any sidecar gRPC method. The importer is a peripheral worker; it writes to `audit_outbox` directly. Reviewer greps for `sidecar`, `RequestDecision`, `CommitEstimated`, `Release` calls in the importer crate and confirms zero matches. |
| `F3` | The audit row's `tenant_id` comes from `rec.workspace_id` only — no rewriting, no fallback to a global default. Reviewer reads the construction. |
| `F4` | When `import_record_to_audit_row` returns a row with `amount_micro_usd = 0` (unknown plan), the row is **still written** (so it appears on the dashboard as unpriced). Reviewer confirms there is no early-return that skips the write for unknown-plan records. |
| `F5` | The CloudEvent envelope is built once per row inside the same loop; `subject = row.tenant_id`, `type = "spendguard.audit.import.genspark_credit"`. |

## 5. Live-mode gating assertions

`COV_87` ships the `live` Cargo feature + `GENSPARK_API_TOKEN` runtime gate. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `L1` | The `live` module is fully `#[cfg(feature = "live")]` at the file level. Reviewer opens `live.rs` and confirms the cfg attribute on the first line. |
| `L2` | `Cargo.toml` `[features]` declares `live = ["dep:reqwest", "dep:secrecy"]` — note the `dep:` prefix, which prevents implicit feature exposure (Rust 2021 / Cargo workspace inheritance correctness). |
| `L3` | Reqwest client has `.timeout(Duration::from_secs(30))` (or similar bound) — never an unbounded timeout. The admin API is external; a hanging request shouldn't block the worker forever. |
| `L4` | The bearer-auth header is set via `.bearer_auth(token.expose_secret())`. Reviewer reads the request builder and confirms no manual `Authorization` header construction (avoids accidentally double-encoding). |
| `L5` | The `from_env` constructor returns distinct error messages for missing-vs-empty-vs-short-token. Reviewer reads the error chain and confirms operator-debuggability. |
| `L6` | The `MIN_TOKEN_LEN` constant is `32`. Documented in design §6 decision #4. Reviewer confirms the constant value. |
| `L7` | The bin `main.rs` exits with non-zero on `--workspace` without `live` feature, with a stderr message containing "live" so the operator understands the gate. Reviewer reads the dispatch logic. |

## 6. Migration assertions

`COV_86` adds migration 0053. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `G1` | Migration number 0053 doesn't collide with existing 0024+ migrations or sibling-deliverable reservations (D13 reserved 0044-0046; D14/D15 may reserve 0047-0049 / 0050-0052; D16 takes 0053). Reviewer runs `ls services/canonical_ingest/migrations/00{4,5}[0-9]*.sql` and confirms exclusive ownership. |
| `G2` | Migration is wrapped in `BEGIN; … COMMIT;` so partial failure is atomic. |
| `G3` | Both CHECK constraint recreations use `DROP CONSTRAINT IF EXISTS … ; ADD CONSTRAINT …` (idempotent). Reviewer confirms the `IF EXISTS` clause. |
| `G4` | The expanded `reservation_source` set is exactly `('byok', 'subscription_meter', 'import_genspark')`. Reviewer cross-references D13/0044's set + the new D16 value. |
| `G5` | The expanded `import_source` set is exactly `(NULL OR 'anthropic_console_usage', 'openai_admin_usage', 'genspark_billing')`. Reviewer cross-references D13/0046's set. |
| `G6` | Partial index `idx_audit_outbox_import_genspark` predicate matches the CHECK value exactly. |
| `G7` | `migration_inventory.toml` updated with the new migration, checksum-pinned per existing convention. |
| `G8` | The down-migration restores the **D13-narrowed** CHECK set, NOT a pre-D13 set. (Removing D13's values would itself break D13 rows.) Reviewer reads the down-script. |

## 7. Fixture + provenance assertions

`COV_86` adds three fixtures + `PROVENANCE.md`. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `P1` | All three fixture files are committed and present. |
| `P2` | `PROVENANCE.md` lists every fixture with capture date, operator initials, source, redaction status. |
| `P3` | No fixture contains a real Genspark workspace ID (`T9`). |
| `P4` | No fixture contains prompt content (`T10`). |
| `P5` | `genspark_usage_unknown_plan.json` has at least one record with `"plan": "enterprise"` (or any string NOT in the price table) — required to exercise the fallback path. |
| `P6` | All `window_start` / `window_end` timestamps are valid RFC3339 strings parseable by `chrono`. |
| `P7` | The Plus-tier fixture's hand-computable expected `amount_micro_usd` aggregate matches what the test asserts. Reviewer recomputes by hand: `sum_credits × 0.001999 × 1e6` and confirms the test's expected constant. |

## 8. Demo / Makefile assertions

`COV_88` adds the demo target. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `D1` | Makefile target follows the existing `demo-verify-*` naming. |
| `D2` | The verifier SQL uses the same `audit_outbox` columns the existing `verify_step_*.sql` files use — no schema drift. |
| `D3` | The demo runtime script does NOT make any outbound HTTP call to `api.genspark.ai` — only fixture mode is exercised. Reviewer reads the script and confirms no `curl`/`wget`/`reqwest` against Genspark. |
| `D4` | `make demo-clean` removes D16-specific artefacts (rows where `reservation_source = 'import_genspark'`). |
| `D5` | The demo target exits cleanly when run twice in a row (idempotent — operator can re-run for debugging). |
| `D6` | The demo runs from a fresh clone with default-features build only (does NOT require `--features live`). |

## 9. Docs assertions

`COV_88` adds a Starlight integration doc page. Reviewer asserts:

| ID | Assertion |
|----|-----------|
| `C1` | The page explicitly states "post-hoc reconciliation, NOT enforcement" above the fold. |
| `C2` | The page links to the strategy memo §"Archetype IV" explaining why enforcement is unreachable. |
| `C3` | The page documents the `GENSPARK_API_TOKEN` env var with its ≥ 32-char requirement. |
| `C4` | The page explains the unknown-plan fallback semantic (row appears unpriced on the dashboard). |
| `C5` | The page documents the CloudEvent type `spendguard.audit.import.genspark_credit`. |
| `C6` | The page links to the operator-side scheduling concern (cron / Kubernetes CronJob example). |
| `C7` | Embedded code/JSON examples are wrapped in `is:raw` (project memory `feedback_astro_is_raw`). |
| `C8` | The page does NOT promise live polling support in CI — explicitly notes that `live` is operator-commissioned. |

## 10. R1-R5 escalation criteria

| Round | Blocker count | Action |
|-------|--------------|--------|
| R1 | 0 → MERGE | none |
| R1 | ≥ 1 → dispatch same implementer with findings | typical 1-3 findings on first review (D16 surface is much narrower than D13: pure converter + price table + migration + feature gate). |
| R2-R4 | drop to 0 → MERGE | follow normal cadence |
| R5 | ≥ 1 Blocker → Staff+ panel arbitration | panel composition per build plan §1.3 |

**R5 panel summarizer override:** Backend Architect (per design §6 locked decision #9). Rationale: the surface is a periodic worker + a price table + a migration. D13's security framing (Authorization parsing, synthetic 429 DoS) does NOT transfer — the only security concern is `GENSPARK_API_TOKEN` handling, and `secrecy::SecretString` plus the `T1`-`T4` assertions cover it cleanly. Architecture framing (cross-importer consistency with D13, dashboard cost-basis correctness, additive-migration discipline) dominates.

## 11. Per-slice review focus

| Slice | Focus areas |
|-------|-------------|
| `COV_84_d16_genspark_crate_scaffold` | §1 (T3, T4), §2 (X8) |
| `COV_85_d16_credit_price_and_record_to_row` | §1 (T6, T7, T15), §2 (X1-X7), §3 (M1-M7), §4 (F3, F4) |
| `COV_86_d16_fixture_import_path` | §1 (T5, T8-T12), §2 (X9), §4 (F1-F2, F5), §6 (G1-G8), §7 (P1-P7) |
| `COV_87_d16_live_http_client` | §1 (T1-T4, T13-T14), §5 (L1-L7) |
| `COV_88_d16_demo_and_docs` | §8 (D1-D6), §9 (C1-C8) |

Each slice review only consults its focus areas plus repo-wide standards; the reviewer is NOT asked to re-check the whole list for every slice.
