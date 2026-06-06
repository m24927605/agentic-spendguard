# D14 — Devin Billing Importer (`spendguard-importer-devin`)

**Status:** Spec — Tier 3, build plan §2.3. **Parent:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md) Archetype IV (unreachable). **Depends on:** [`D13`](../D13_subscription_meter/design.md) — reuses `ReservationSource` proto enum + `audit_outbox.import_source` column. **Owner:** Backend Architect.

## 1. Problem

Devin (Cognition Labs) runs the agent loop entirely inside Cognition's cloud VM. Customer network never carries the per-LLM-call payload; egress proxy + SDK adapters intercept nothing. The only telemetry exposed is post-hoc — Devin Team API usage in **ACU (Agent Compute Unit, ≈ $2.25/ACU)**. Without an importer, Devin spend is invisible on the dashboard and the CIO / CFO single-pane-of-glass view collapses.

## 2. Goals / non-goals

**In:** stub `spendguard-importer-devin` crate (D13 importer pattern, `live` off); commit `spendguard.audit.import.devin_acu` CloudEvent schema; fixture-driven import replays `devin_usage.json`, converts ACU → $ via price table, emits synthetic audit events with `reservation_source = subscription_meter`, `import_source = devin_team_api`; live HTTP client behind `live` feature gated by `DEVIN_API_TOKEN`; demo `import_devin_fixture`; docs framing "reconciliation only, no gating."

**Out:** live gating (architecturally impossible); Manus / Genspark importers (D15 / D16); backfill beyond Team API retention; real-time streaming (poll-based, default hourly).

## 3. Why this is feasible without live API access

Per build plan §3 for unreachable deliverables: feasibility = "billing-importer testable against a vendor-staged fixture; acceptance is 'synthetic audit event emitted', not 'live import succeeded.'" D14 ships (a) sanitized `tests/fixtures/devin_usage.json`, (b) pure `import_record_to_audit_row` (no I/O), (c) `live` behind a Cargo feature — CI flips it only when `DEVIN_API_TOKEN` is set; absence = "fixture is the merge gate."

## 4. Architecture

```
poll cycle (cron / Helm cronjob)
  └─► DevinClient (live feat) ──┐                    ┌─ acu_price_table
                                │                    │   (loader + cache)
      FixtureLoader (default) ──┴─► ImportRecord ────┴─► import_record_to_audit_row
                                                          (ACU → micro_usd + envelope)
                                                                │
                                                                ▼
                                                  canonical_ingest::AppendEvents
                                                                │
                                                                ▼
                                                  audit_outbox row,
                                                  import_source='devin_team_api'
```

Fixture mode swaps `DevinClient` for `FixtureLoader` — same `ImportRecord`, identical downstream path. No branching after the loader boundary.

### 4.1 Devin API surface (re-verified at impl time)

| Endpoint | Purpose |
|----------|---------|
| `GET /api/v1/teams/{team_id}/usage?start=&end=` | Aggregate ACU per session — primary poll |
| `GET /api/v1/sessions?team_id=&since=` | Cursor-based incremental session list |
| `GET /api/v1/sessions/{id}/acu-consumption` | Per-action ACU breakdown — opt-in detail |

`live` codepath is feature-gated so endpoint changes don't break stub build.

### 4.2 ACU → $ conversion

Price table at `services/importer_devin/assets/devin_acu_prices.json` (`pricing_version`, `effective_from`, `rates[]` of `{plan, usd_per_acu}`). Conversion: `amount_micro_usd = round(acu_consumed * usd_per_acu * 1_000_000)`. **Enterprise plans without published rate** emit `amount_micro_usd = NULL` + `reason_code = "devin_enterprise_negotiated_rate"` — dashboards distinguish "unknown rate" from "zero spend". `pricing_version` stamped on every row; rate back-revision does not mutate history.

### 4.3 CloudEvent schema — `spendguard.audit.import.devin_acu`

Committed at sibling `cloudevent-schema.md`. Envelope mirrors `spendguard.audit.*` family (decision / outcome / fail_policy_admit):

```
specversion:    1.0
type:           spendguard.audit.import.devin_acu
source:         spendguard-importer-devin
id:             <uuidv7>
time:           <iso8601>
subject:        tenant/<tenant_id>/devin/team/<dt>/session/<ds>
datacontenttype application/json
data: {
  schema_version: "v1alpha1",
  tenant_id, budget_id,
  devin_team_id, devin_session_id,
  acu_consumed, usd_per_acu,
  amount_micro_usd, pricing_version,
  window_start, window_end,
  reservation_source: "subscription_meter",
  import_source:      "devin_team_api",
  ingestion_mode:     "fixture" | "live",
  fixture_provenance_sha256:  // null when live
}
```

`schema_version` is the contract handle. Additive fields land as `v1alpha2`; importer emits highest known version. Downstream default-zeroes unknown fields per the project's additive-only convention.

## 5. Slices

| # | Slice | Size | Scope |
|---|-------|------|-------|
| `COV_67` | `d14_devin_crate_scaffold` | S | Crate `services/importer_devin/`, Cargo.toml, `lib.rs`, `live` flag (no deps), `publish = false`, workspace member. |
| `COV_68` | `d14_devin_acu_price_table` | M | `acu_price_table.rs` + asset `assets/devin_acu_prices.json`. Pure `acu_to_micro_usd`. Tests: round-trip, enterprise-NULL, version stamping. |
| `COV_69` | `d14_devin_fixture_import_path` | M | `ImportRecord` + `import_record_to_audit_row` + `FixtureLoader`. Mig 0047 widens `import_source` CHECK. Contract tests + PG round-trip. |
| `COV_70` | `d14_devin_cloudevent_schema_doc` | S | `cloudevent-schema.md` sibling + `cloudevent_envelope.rs` builder + golden envelope test. |
| `COV_71` | `d14_devin_live_client_behind_feature` | M | `live` pulls `reqwest` (rustls-only). `DevinClient` + retry + 401/403/429. `wiremock` tests. Clear error when `DEVIN_API_TOKEN` unset. |
| `COV_72` | `d14_devin_demo_and_docs` | M | `make demo-verify-import-devin-fixture` + verifier SQL + Starlight integration page + README adapter row. |

## 6. Locked decisions

1. **Reuse D13's `import_source` column** (mig 0046). Mig 0047 widens `CHECK` to add `'devin_team_api'` (additive, COV_69).
2. **`reservation_source = subscription_meter`** — Devin is post-hoc settled by Cognition; never write `ledger_entries`. Same fork as D13.
3. **Fixture-first** — primary merge gate is fixture-driven; live is feature-gated and CI-optional.
4. **Pricing version stamped on every row** — rate back-revision does not mutate history.
5. **Enterprise NULL rate is explicit** — `amount_micro_usd = NULL` + `reason_code = "devin_enterprise_negotiated_rate"`.
6. **R5 panel summarizer: Backend Architect** — schema + import-pipeline correctness dominates; no DoS, no proxy-edge auth parsing.
7. **`live` feature off by default** — default `cargo tree` shows no `reqwest`/`hyper-tls`.
8. **CloudEvent `type` = `spendguard.audit.import.devin_acu`** — `import.<vendor>_<unit>` subnamespace convention for D14/D15/D16.
9. **Synthetic fixture IDs only** — `TEAM_FIXTURE_001`, `SESSION_FIXTURE_001`. PROVENANCE.md pins generator SHA-256.
10. **Importer is idempotent** — key = `(devin_team_id, devin_session_id, window_end)`. Canonical_ingest dedupes via existing event_id replay. Re-running the same window must not double-emit.
