# spendguard-importer-manus

Manus (Butterfly Effect, Meta-acquired 2026) billing importer for
SpendGuard. Reconciliation only — SpendGuard cannot gate Manus
sessions because the agent loop runs inside a vendor-managed VM.

## What this is

A Rust crate + binary that:

1. Pulls credit usage from the Manus admin REST API (`/v1/usage`),
   gated behind the optional `live` Cargo feature so the default
   build is HTTP-free.
2. Converts vendor credits to estimated USD via the vendored TOML
   price table at `assets/price_table.toml` (integer micro-USD,
   saturating multiply).
3. Emits signed `spendguard.audit.import.manus_credit` CloudEvents
   tagged with `reservation_source=subscription_meter` (D14/D16-aligned)
   and `import_source=manus_team_api`.

The default merge gate is the **fixture-driven** path — replay the
sanitized `tests/fixtures/manus_usage.json` snapshot through the
importer. Live mode is operator-opt-in and requires `MANUS_API_TOKEN`.

## Two operating modes

### Fixture mode (default merge gate)

```bash
cargo run -p spendguard-importer-manus --bin importer_manus -- \
    --mode fixture \
    --fixture services/importer_manus/tests/fixtures/manus_usage.json
```

The binary prints one CloudEvent per terminal session to stdout. The
demo runner (`deploy/demo/import_manus_fixture_demo.sh`) wires the
INSERT path against a throwaway postgres.

### Live mode (operator-opt-in, gated by `MANUS_API_TOKEN`)

```bash
export MANUS_API_TOKEN=<Team+ admin API token>
export MANUS_API_BASE_URL=https://api.manus.ai   # optional override

cargo run -p spendguard-importer-manus --features live --bin importer_manus -- \
    --mode live
```

The `live` feature pulls `reqwest` with `rustls-tls` only (no
`native-tls`, no `openssl-sys`), uses a 30s per-request timeout, and
bounded exponential backoff (cap: 1 hour) for rate-limit resilience.

Scope the token to **Team+ admin API** only — least privilege.

## Three pricing tiers

| Tier               | `credit_cost_micro_usd` | Audit row behaviour |
|--------------------|-------------------------|----------------------|
| `team_plan`        | `20_526`                | $39/mo / 1900 credits = $0.020526/credit; integer-truncated. Headline conversion: 47 credits × 20_526 = 964_722 micro-USD. |
| `enterprise`       | `0` (default)           | Operator override required at deploy time. Dashboards show NULL spend until override lands. |
| `enterprise_byok`  | `0` (LOAD-BEARING)      | BYOK tier customers pay the LLM provider directly. The importer MUST NOT double-bill them. NEVER raise without re-reading the BYOK contract. |

## Why we cannot gate

Manus runs each agent task entirely inside a vendor-managed VM. The
customer network never sees the per-LLM-call payload; the egress
proxy + SDK adapters intercept nothing. The admin API exposes only
post-hoc aggregate credit usage, not a pre-call hook.

This is **Archetype IV** in the SpendGuard framework-coverage
analysis. Gating is architecturally impossible without Manus shipping
a pre-call webhook.

See [`docs/specs/coverage/D15_manus_importer/design.md`](../../docs/specs/coverage/D15_manus_importer/design.md)
for the design rationale and slice-level breakdown.

## Test surface

- 97 library unit tests (default-features lib + live-feature live module).
- 4 CloudEvent envelope golden tests (`tests/cloudevent_envelope_golden.rs`).
- 15 fixture round-trip integration tests (`tests/fixture_round_trip.rs`).

Run all with `cargo test -p spendguard-importer-manus --features live`.

## Idempotency

Re-running the same window does not double-emit. The CloudEvent
`event.id` is a deterministic UUIDv5 derived from
`(workspace_id, session_id, window_end)`; the audit-row `dedupe_key`
is `manus:<session_id>` so canonical_ingest dedups via the existing
`event_replay_dedup` table regardless of which path emitted the row.

## Layout

```
services/importer_manus/
├── Cargo.toml                       # publish=false, live Cargo feature
├── README.md                        # this file
├── assets/
│   └── price_table.toml             # tier -> micro-USD/credit
├── scripts/
│   └── generate_fixture.py          # regenerates manus_usage.json
├── src/
│   ├── lib.rs                       # module re-exports
│   ├── record.rs                    # UsageRecord + ImportRecord + Tier + Status
│   ├── pricing.rs                   # PriceTable + credit_to_usd_micros
│   ├── fixture.rs                   # FixtureLoader (default merge gate)
│   ├── audit.rs                     # import_record_to_audit_row + constants
│   ├── error.rs                     # ImporterError + MeterError
│   ├── cloudevent_envelope.rs       # CloudEvent 1.0 envelope builder
│   ├── bin/
│   │   └── importer_manus.rs        # binary entrypoint (--mode fixture|live)
│   └── live/                        # #[cfg(feature = "live")]
│       ├── mod.rs
│       ├── client.rs                # ManusClient (rustls-only reqwest)
│       ├── errors.rs                # LiveError (typed 401/403/429/5xx)
│       └── poll_loop.rs             # PollConfig + bounded backoff
└── tests/
    ├── fixtures/
    │   ├── manus_usage.json         # 8 sessions × 3 tiers (1 in_progress filtered)
    │   └── PROVENANCE.md            # script + body SHA-256 pins
    ├── golden/
    │   ├── cloudevent_v1alpha1_team_fixture.json
    │   ├── cloudevent_v1alpha1_enterprise_byok_fixture.json
    │   └── cloudevent_v1alpha1_team_live.json
    ├── fixture_round_trip.rs        # demo-path emission count + math
    └── cloudevent_envelope_golden.rs # byte-equal envelope pin
```
