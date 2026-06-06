# D16 — Genspark Billing Importer (`spendguard-importer-genspark`)

**Status:** Spec — Tier 3, build plan §2.3. **Parent:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md) Archetype IV — fully-managed cloud agent. **Sibling pattern:** [`D13`](../D13_subscription_meter/design.md) §5 importer stubs. **Owner:** Backend Architect.

## 1. Problem

Genspark Super Agent runs inside Genspark's cloud VM. Operators buy a subscription (Plus $19.99/mo, Pro $24.99/mo, Premium $249.99/mo); every action draws credits and Genspark settles dollars internally. The client sees a task result + an aggregate credit number — never a per-LLM-call payload, never a tokenized prompt. Pattern 2 (`OPENAI_BASE_URL`) and Pattern 3 (`HTTPS_PROXY` + CA) are unreachable: LLM calls happen inside Genspark's VM. SpendGuard cannot intercept, tokenize, reserve, or enforce. The only reachable surface is post-hoc reconciliation via Genspark's higher-tier admin API. D16 polls it, converts credits → USD, emits synthetic audit events tagged `reservation_source = 'import_genspark'`, surfaces Genspark spend on the CIO / CFO single-pane dashboard.

## 2. Goals / non-goals

**In:** new Rust crate `spendguard-importer-genspark` (`services/importer_genspark/`); fixture-driven import (recorded `genspark_usage.json`); credit → USD via versioned `genspark_credit_price.toml`; audit row `reservation_source = 'import_genspark'` + `import_source = 'genspark_billing'`; CloudEvent `spendguard.audit.import.genspark_credit`; live HTTP polling behind `live` Cargo feature + `GENSPARK_API_TOKEN`; demo `import_genspark_fixture`; Starlight integration doc.

**Out:** live enforcement (vendor owns quota); reverse-engineering Genspark's agent protocol (Archetype III, SOW); intercepting in-VM LLM calls (unreachable); free-tier (no admin API). Production polling cadence is operator-wired (cron / CronJob).

## 3. Architecture

```
   ┌──────────────────────────────────────┐
   │ Genspark Admin API (higher tier)     │
   │  GET /v1/admin/usage?workspace=…     │
   └────────────┬─────────────────────────┘
                │ live mode (GENSPARK_API_TOKEN)
                ▼
 fixture ─► ┌─────────────────────────────────┐
 (default)  │ importer_genspark::import_window│
            │ 1. fetch records (live|fixture) │
            │ 2. credit→USD via price table   │
            │ 3. emit audit row + CloudEvent  │
            └────────────┬────────────────────┘
                         ▼
   ┌──────────────────────────────────────────┐
   │ audit_outbox (mig 0053 extends 0046)     │
   │  reservation_source = 'import_genspark'  │
   │  import_source      = 'genspark_billing' │
   └──────────────────────────────────────────┘
```

`fixture` mode (default) reads `services/importer_genspark/tests/fixtures/genspark_usage.json`. `live` mode (Cargo-feature-gated) polls the admin API. Both shapes converge on the same `import_record_to_audit_row` pure function — identical to D13's `importer_anthropic` / `importer_openai` contract.

### 3.1 Credit → USD conversion

Genspark prices credits at the subscription tier, not per credit. Conversion uses a versioned static table `genspark_credit_price.toml`:

| Plan | Monthly $ | Monthly credit grant | $ / credit |
|------|-----------|---------------------|------------|
| Plus | 19.99 | 10,000 | 0.001999 |
| Pro | 24.99 | 12,500 | 0.001999 |
| Premium | 249.99 | 125,000 | 0.001999 |

The table is committed source (matches D13's pricing-snapshot convention) and carries `pricing_version` into every audit row. Operators override via `GENSPARK_PRICE_TABLE_PATH`. Unknown plan → `amount_micro_usd = 0` + `reason_code = "genspark_plan_unknown"`, surfaced on the dashboard as unpriced (never silently mis-priced).

### 3.2 Audit row + CloudEvent

`audit_outbox.reservation_source = 'import_genspark'` (new enum value, mig 0053) and `import_source = 'genspark_billing'` (CHECK value extending D13/0046, mig 0053). CloudEvent envelope `type = "spendguard.audit.import.genspark_credit"` routes Genspark rows distinctly. The importer **must not** write `ledger_entries` or `reservations` — the operator pre-paid the subscription; reserving would double-count (same constraint as D13 §4.3).

### 3.3 Live mode gating

`live` Cargo feature pulls `reqwest` (rustls-tls) + `serde` JSON. Default build is HTTP-free (verified via `cargo tree -e=normal`, mirrors D13 `A10.3`). Runtime gate: `GENSPARK_API_TOKEN` must be present, non-empty, and ≥ 32 chars (rejects `TODO` placeholders). Token read once at startup into `secrecy::SecretString`; never logged.

## 4. Fixtures

`services/importer_genspark/tests/fixtures/genspark_usage.json` is a recorded admin-API response, workspace IDs redacted to `FAKE_ws_*`, credit numbers synthetic. `PROVENANCE.md` pins capture date, redaction script SHA-256, no-PII assertion (admin API returns credit-line aggregates only, no prompt content). Three variants: `genspark_usage.json` (Plus, single workspace, 7-day window), `genspark_usage_premium.json` (Premium, multi-workspace), `genspark_usage_unknown_plan.json` (forces fallback).

## 5. Slices

5 slices: `COV_84_d16_genspark_crate_scaffold` (S); `COV_85_d16_credit_price_and_record_to_row` (M, price loader + pure `import_record_to_audit_row` + contract tests); `COV_86_d16_fixture_import_path` (M, `import_window_from_fixture` + test PG); `COV_87_d16_live_http_client` (M, `live` feature + reqwest + token gating, no live CI); `COV_88_d16_demo_and_docs` (S, demo target + Starlight doc + README row).

## 6. Locked decisions

1. `reservation_source = 'import_genspark'` is **distinct** from D13's `subscription_meter` — different cost basis (vendor aggregate vs. SpendGuard tokenizer estimate); dashboard must split.
2. Importer **never** writes `ledger_entries` or `reservations` — subscription is pre-paid.
3. `live` feature OFF by default; default build is HTTP-free.
4. `GENSPARK_API_TOKEN` is a runtime gate on top of the compile-time `live` gate; < 32 chars → refuse to start.
5. CloudEvent type `spendguard.audit.import.genspark_credit` is namespaced under `spendguard.audit.import.*` (sibling D14/D15 will use `…import.devin_acu` / `…import.manus_credit`).
6. Credit pricing table is **committed source**, not runtime-fetched — spec drift caught by tests, never by silent runtime change.
7. Unknown plan → `amount_micro_usd = 0` + `reason_code = "genspark_plan_unknown"`; never silently mis-priced.
8. Mig 0053 is **additive** to D13/0046's `import_source` CHECK — drops + re-creates with the new value, preserves rows.
9. R5 panel summarizer: Backend Architect (no inbound auth parsing, no synthetic-429 DoS surface — D13's security framing does not transfer).
10. Importer is a **periodic worker**, not a daemon: `cargo run --bin … -- --window-from <ts> --window-to <ts>`; scheduling is operator-wired.
