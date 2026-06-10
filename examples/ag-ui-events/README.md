# `examples/ag-ui-events/` — SpendGuard AG-UI spend-event demo runner

> **Display-only.** AG-UI events are a presentation surface. SpendGuard
> enforcement happens in the SpendGuard adapters and sidecar before the
> provider call; these events report decisions already made and can neither
> grant nor deny spend.

Demo runner for coverage deliverable D39 (`DEMO_MODE=ag_ui_events`). It drives
a REAL SpendGuard run against the sidecar UDS — handshake → `reserve` (ALLOW)
→ provider call against the in-network counting-stub → `commitEstimated` →
`reserve` over the seeded hard-cap (denied by the **sidecar**, pre-dispatch) —
and renders each decision the enforcement plane made as a `spendguard.*`
AG-UI `CUSTOM` event:

1. `spendguard.budget.snapshot` — seeded budget state (env-passed seed values,
   cross-checked against the ledger by the verify gate).
2. `spendguard.reservation.created` — from the real `DecisionOutcome`
   (`decision_id` / `reservation_id` / TTL straight off the RPC; nothing is
   fabricated).
3. `spendguard.reservation.committed` — after `commitEstimated(SUCCESS)`.
4. `spendguard.decision.denied` — from the real denied-decision error; the
   runner additionally asserts the counting-stub hit counter did NOT move,
   proving the deny happened at the sidecar before any provider dispatch
   (the AG-UI event merely reports it).

`spendguard.reservation.released` is fixture/unit-tested only in v0.1.0 — the
demo's deny step never creates a reservation to release (design.md §11.9,
documented non-gap).

The recorded `encodeSse(...)` frames are then replayed over HTTP:

- `GET :8077/healthz` → `200 ok` (the compose healthcheck — only healthy
  after all 4 steps succeeded)
- `GET :8077/events` → all frames in emission order, then close

The hard gate (`make demo-verify-ag-ui-events`) captures `/events` and
asserts the exact 4-frame sequence, every required field, canonical-bytes
round-trip, and the SSE↔ledger `reservation_id` join. See
`deploy/demo/ag_ui_events/` and `docs/specs/coverage/D39_ag_ui/design.md` §9.

These events are **unsigned UI hints** — NOT the SpendGuard audit chain. The
signed audit chain lives in the sidecar/ledger.

## Run it

```bash
make demo-up DEMO_MODE=ag_ui_events     # from the repo root
make demo-verify-ag-ui-events
make demo-down
```

## Env

`SPENDGUARD_SIDECAR_UDS`, `SPENDGUARD_TENANT_ID`, `SPENDGUARD_BUDGET_ID`,
`SPENDGUARD_WINDOW_INSTANCE_ID`, `SPENDGUARD_UNIT_ID`,
`SPENDGUARD_PRICING_VERSION`, `SPENDGUARD_DEMO_OPENING_BALANCE_ATOMIC`,
`SPENDGUARD_COUNTING_STUB_URL` — set by the compose overlay
(`deploy/demo/ag_ui_events/docker-compose.yaml`), mirroring the langchain_ts
runner. The pricing freeze tuple (`SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX`, FX +
unit-conversion versions) is sourced from the bundles `runtime.env`
(HARDEN_D05_WI convention).
