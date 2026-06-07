# spendguard-importer-genspark

**Status:** D16 (COV_84-88) — reconciliation-only billing importer for
the Genspark Super Agent.

## Why this exists

Genspark Super Agent runs the agent loop entirely inside Genspark's
cloud VM. The customer's network never carries the per-LLM-call
payload; SpendGuard's egress proxy + SDK adapters intercept nothing.
The only feasible integration is **post-hoc reconciliation** — pull
the bill after the fact via the admin usage API, surface it on the
dashboard, alert on threshold crossings.

This crate **does not gate** Genspark sessions. That contract belongs
to Genspark. See [`docs/strategy/framework-coverage-2026-06.md`](../../docs/strategy/framework-coverage-2026-06.md)
§"Archetype IV" for the architectural reasoning.

## Layout

```
services/importer_genspark/
├── Cargo.toml
├── README.md                              # this file
├── assets/
│   └── genspark_credit_prices.json        # versioned price table
├── scripts/
│   └── generate_fixture.py                # regenerate the canonical fixture
├── src/
│   ├── lib.rs                             # public API
│   ├── bin/importer_genspark.rs           # CLI entrypoint
│   ├── credit_price_table.rs              # price loader + credit→USD
│   ├── import_record.rs                   # pure import_record_to_audit_row
│   ├── cloudevent_envelope.rs             # CloudEvent 1.0 builder
│   ├── fixture_loader.rs                  # default-merge-gate reader
│   └── live/                              # feature-gated `live` HTTP client
│       ├── mod.rs
│       ├── client.rs                      # rustls-only reqwest wrapper
│       ├── errors.rs                      # typed Display-sanitized errors
│       └── poll_loop.rs                   # bounded backoff helper
└── tests/
    ├── fixtures/
    │   ├── genspark_usage.json            # canonical sanitized snapshot
    │   └── PROVENANCE.md                  # capture provenance + SHA-256 pins
    ├── golden/
    │   ├── cloudevent_v1alpha1_plus_fixture.json
    │   ├── cloudevent_v1alpha1_unknown_plan_fixture.json
    │   └── cloudevent_v1alpha1_premium_live.json
    ├── cloudevent_envelope_golden.rs      # byte-equal envelope check
    └── fixture_round_trip.rs              # fixture replay invariants
```

## Build

The crate is workspace-excluded; build it directly:

```bash
cargo build --manifest-path services/importer_genspark/Cargo.toml
cargo test  --manifest-path services/importer_genspark/Cargo.toml
```

Default build is HTTP-client-free:

```bash
cargo tree -e=normal --manifest-path services/importer_genspark/Cargo.toml | grep reqwest
# → no output
```

Live mode pulls `reqwest` + `tokio` + `url` + `tracing`:

```bash
cargo build  --manifest-path services/importer_genspark/Cargo.toml --features live
cargo test   --manifest-path services/importer_genspark/Cargo.toml --features live
```

## Run

### Fixture mode (default)

```bash
cargo run --manifest-path services/importer_genspark/Cargo.toml \
  --bin importer_genspark -- \
  --mode fixture \
  --fixture services/importer_genspark/tests/fixtures/genspark_usage.json \
  --tenant demo \
  --budget genspark-budget
```

The binary prints one CloudEvent envelope per fixture row to stdout
and a count summary to stderr.

### Live mode

```bash
export GENSPARK_API_TOKEN=<Admin API token, at least 32 chars>
cargo run --manifest-path services/importer_genspark/Cargo.toml \
  --bin importer_genspark --features live -- \
  --mode live \
  --tenant demo \
  --budget genspark-budget
```

### Demo

```bash
make -C deploy/demo demo-verify-import-genspark-fixture
```

Spins up a throwaway `postgres:16-alpine`, applies mig 0061, runs the
importer in fixture mode, INSERTs the envelopes as `audit_outbox`
rows, and asserts the invariants in
`deploy/demo/verify_step_import_genspark_fixture.sql`.

## Locked invariants

* `data.reservation_source == "subscription_meter"` — never `byok`.
* `data.import_source == "genspark_team_api"` — matches mig 0061.
* Default `cargo tree -e=normal` is HTTP-client-free.
* `import_record_to_audit_row` is pure (no I/O, no clock).
* Idempotency key `(workspace_id, task_id, window_end)` produces a
  deterministic UUIDv5 `event.id`.
* Unknown plan slug → `amount_micro_usd = 0` +
  `reason_code = "genspark_plan_unknown"` (BOTH fields set).
* `GENSPARK_API_TOKEN` runtime gate: distinct error variants for
  missing / empty / too-short (< 32 chars).
* Fixture loader hard-rejects non-`FAKE_ws_NNN` / non-`FAKE_task_NNN`
  identifiers at parse time.

## Spec

* [Design](../../docs/specs/coverage/D16_genspark_importer/design.md)
* [Implementation](../../docs/specs/coverage/D16_genspark_importer/implementation.md)
* [Review standards](../../docs/specs/coverage/D16_genspark_importer/review-standards.md)
