# D15 — Manus Billing Importer (`spendguard-importer-manus`)

**Status:** Spec — Tier 3, build plan §2.3. **Parent:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md) Archetype IV. **Sibling:** [`D13`](../D13_subscription_meter/design.md) §5 stub-crate pattern; identical shape to upcoming D14 Devin / D16 Genspark importers. **Owner:** Backend Architect.

## 1. Problem

Manus (Butterfly Effect, Meta-acquired 2026) runs each agent task inside a vendor-managed VM. Clients see a task ID and a **credit** counter — never per-LLM-call payloads, never a model split. No proxy hook, no callback bus, no base-URL swap. Archetype IV is architecturally unreachable for predictive gating; the only legible signal is the post-hoc Team+ admin REST surface gated by an API token. Without D15 the CIO/CFO single-pane-of-glass story has a Manus-shaped hole next to the Devin-shaped one.

## 2. Goals / non-goals

**In:** new crate `services/importer_manus/` shipping a **fixture-driven** primary path — `manus_usage.json` → synthetic `audit_outbox` rows tagged `reservation_source = 'import_manus'`, `import_source = 'manus_admin_usage'`. Credit→USD via `manus_price_table` (three tiers: `team_plan` $39/mo ≈ 1900 credits → 20_526 micro-USD/credit; `enterprise` operator-overridden; `enterprise_byok` $0 because the customer pays the LLM provider direct). HTTP client behind opt-in `live` Cargo feature gated by `MANUS_API_TOKEN` env var. CloudEvent `spendguard.audit.import.manus_credit`. Demo `import_manus_fixture`. Two additive migrations 0047/0048 extending the D13 CHECK convention with forward-compat slots for `import_devin` / `import_genspark` / `*_admin_usage`.

**Out:** live enforcement (vendor settles internally, same reasoning as D13 Archetype II); per-LLM-call attribution (admin API exposes only aggregate credits); D14 Devin and D16 Genspark importers (siblings); live integration tests against `api.manus.ai` (requires paid Team+ tenant); reverse-engineered session-detail extraction.

## 3. Architecture

```
fixture path (default, merge gate):
  manus_usage.json
    → fixture::load_fixture()   ── tier/status validation
    → pricing::credit_to_usd_micros()   ── saturating i64 math
    → audit::import_record_to_audit_row()   ── tag reservation_source/import_source
    → canonical_ingest::append_audit_outbox()
    → outbox_forwarder emits CloudEvent spendguard.audit.import.manus_credit

live path (opt-in --features live, gated by MANUS_API_TOKEN):
  live::LiveClient::from_env() → poll_usage(since, until) → same downstream pipeline
```

Same fork as D13 §4: the importer **never** writes `ledger_entries`, **never** holds a reservation, **never** calls `sidecar::request_decision`. Pure post-hoc reconciliation.

### 3.1 Manus admin REST surface

`GET /v1/usage?since=&until=&cursor=` with `Authorization: Bearer ${MANUS_API_TOKEN}` → aggregate credit usage per session. Fields consumed: `session_id`, `workspace_id`, `tier`, `credits_consumed`, `status`, `started_at`, `completed_at`. Extras ignored (forward-compat). `GET /v1/sessions/{id}` is not used; aggregate is sufficient.

### 3.2 Credit → USD

`assets/price_table.toml` embedded via `include_str!`. Integer micro-USD per credit, no f64 in the hot path. Unknown tier → `Err(MeterError::UnknownTier)` → skipped with WARN. Never invent an amount.

### 3.3 `reservation_source` family

D13 introduced `reservation_source IN ('byok','subscription_meter')`. D15 extends with the `import_*` namespace, listed in mig 0047 CHECK alongside forward-compat slots `'import_devin'` (D14) and `'import_genspark'` (D16). All `import_*` rows are no-reservation, no-ledger. `import_source` parallels with `'manus_admin_usage'` added to mig 0048 CHECK alongside forward-compat `'devin_admin_usage'` / `'genspark_admin_usage'`.

### 3.4 Live mode gating

`live::poll_usage` compiles only with `--features live`. At runtime, empty/missing `MANUS_API_TOKEN` → `Err(MissingToken)`, no HTTP issued. Default build pulls zero HTTP deps (verified by `cargo tree`). Live tests use `httpmock`, never the real vendor.

## 4. Slices (5)

- `COV_70_d15_crate_scaffold` (S) — `Cargo.toml`, lib skeleton, `UsageRecord`/`ImportRecord` types, `publish=false`, `live` feature flag declared optional.
- `COV_71_d15_credit_price_table` (S) — `assets/price_table.toml`, `credit_to_usd_micros` pure fn, `MeterError::UnknownTier`, unit + overflow guard.
- `COV_72_d15_fixture_import_path` (M) — `fixture::load_fixture`, `import_record_to_audit_row`, fixture JSON (8 sessions × 3 tiers + edge cases), contract test vs mig 0048 CHECK.
- `COV_73_d15_live_http_client` (M) — `live::LiveClient` behind `--features live`, cursor-pagination loop with hard upper bound, `httpmock` tests.
- `COV_74_d15_audit_emission_and_demo` (M) — migrations 0047/0048, CloudEvent registration, `demo-import-manus-fixture` target, verifier SQL, docs page.

## 5. Locked decisions

1. **`reservation_source = 'import_manus'`** — distinct value, not overloaded onto `subscription_meter`. Dashboards filter importer rows separately (importer = daily-poll, meter = per-call).
2. **Fixture-driven is the merge gate.** Live mode opt-in `--features live` + runtime `MANUS_API_TOKEN`; live tests gated `#[ignore] #[cfg(feature = "live")]`, NOT a CI gate.
3. **CloudEvent `spendguard.audit.import.manus_credit`** — family pattern `spendguard.audit.import.<vendor>_<unit>` (D14 → `…devin_acu`, D16 → `…genspark_credit`).
4. **R5 panel summarizer: Backend Architect** — narrow security surface (no inbound traffic, no synthetic 429, no tenant resolution). Security Engineer remains on the panel and reviews §1 threat assertions.
5. **Migrations 0047/0048 enumerate forward-compat slots** for `import_devin` / `import_genspark` / `*_admin_usage` — D14/D16 must not need follow-up CHECK migrations.
6. **Unknown tier = skip + WARN.** Never fabricate a USD amount; the dashboard shows zero until the operator updates the price table.
7. **No `ledger_entries`, no `reservations` write.** Importer talks only to `canonical_ingest`'s audit-outbox append API.
8. **No PII in fixtures.** Workspace/session IDs sentinel-prefixed `ws_FAKE_…` / `ses_FAKE_…`; `PROVENANCE.md` pins redaction-script SHA-256.
9. **`publish = false`** — internal until Meta-acquisition vendor SDK stability settles.
10. **Synthetic `model = "manus.session/credit"`**, `input_tokens = output_tokens = 0`. Honest zero beats guessed tokens that would corrupt Strategy A predictions.
11. **`dedupe_key = format!("manus:{session_id}")`** — vendor-prefix isolates D15 from D14/D16 dedupe space.
12. **Integer micro-USD only** in the conversion hot path; saturating multiply guards i64 overflow.
