# `@spendguard/ag-ui`

> **Display-only.** AG-UI events are a presentation surface. SpendGuard
> enforcement happens in the SpendGuard adapters and sidecar before the
> provider call; these events report decisions already made and can neither
> grant nor deny spend.

SpendGuard spend-event family for [AG-UI](https://github.com/ag-ui-protocol/ag-ui) —
pure builders for `spendguard.*`-namespaced AG-UI `CUSTOM` events that let an
agent frontend render budget state, the reservation lifecycle, and deny
reasons that the SpendGuard enforcement plane (adapters + sidecar) already
decided.

These events are **unsigned UI hints**. They are NOT the SpendGuard audit
chain and MUST NOT be treated as audit records — the signed audit chain lives
in the sidecar/ledger, not here.

## Status

`0.1.0` — first release. Slice 1 of coverage deliverable D39
([spec set](https://github.com/m24927605/agentic-spendguard/tree/main/docs/specs/coverage/D39_ag_ui)).

## Install

```bash
npm install @spendguard/ag-ui
```

Zero runtime dependencies. `@ag-ui/core` is an **optional** peer — install it
only if you want AG-UI's own `CustomEvent` typing/validation on your side;
the builders never import it.

## The five events

| Builder | AG-UI CUSTOM `name` | Emitted when |
|---|---|---|
| `buildBudgetSnapshot` | `spendguard.budget.snapshot` | Host app wants to render current budget state |
| `buildReservationCreated` | `spendguard.reservation.created` | Sidecar `reserve` returned ALLOW / ALLOW_WITH_CAPS |
| `buildReservationCommitted` | `spendguard.reservation.committed` | `commitEstimated` acked |
| `buildReservationReleased` | `spendguard.reservation.released` | `release` acked (abort / timeout / cancel path) |
| `buildDecisionDenied` | `spendguard.decision.denied` | Sidecar denied the call pre-dispatch |

Payload field names reuse the ASP Draft-01 vocabulary verbatim wherever the
concept overlaps (`budget_id`, `reservation_id`, `decision_id`,
`reason_codes`, `ttl_expires_at`, `event_time`, `*_atomic` decimal strings).
Every payload carries `schema_version: "1"`.

## Quickstart

```ts
import { buildReservationCreated, encodeSse } from "@spendguard/ag-ui";
import type { DecisionOutcome } from "@spendguard/sdk";

// Inputs come from your SpendGuard adapter's DecisionOutcome — the builders
// never mint IDs, never read clocks, never touch the network. In a real
// adapter these three values arrive from `client.reserve(...)` and your
// SpendGuard configuration (e.g. the SPENDGUARD_* env vars).
declare const outcome: DecisionOutcome;
declare const windowInstanceId: string;
declare const unitId: string;

const reservationId = outcome.reservationIds[0];
if (reservationId === undefined) {
  throw new Error("reserve() returned no reservation_id");
}

const event = buildReservationCreated(
  {
    decisionId: outcome.decisionId,
    reservationId,
    budgetId: "budget-dev-monthly",
    windowInstanceId,
    unit: "usd_micros",
    unitId,
    amountAtomicReserved: "1000000",
    decision: "ALLOW", // SpendGuard wire: CONTINUE → ALLOW; DEGRADE → ALLOW_WITH_CAPS
    ttlExpiresAt: "2026-06-10T08:00:00Z",
    eventTime: "2026-06-10T07:59:58Z",
  },
  { timestampMs: Date.now() }, // optional AG-UI envelope timestamp — caller-supplied
);

const frame = encodeSse(event); // "data: {...canonical JSON...}\n\n"
```

(`@spendguard/sdk` here is only the type import for the example — the
package itself depends on neither `@spendguard/sdk` nor `@ag-ui/core` at
runtime.)

Builders are **pure**: same input, same bytes — the canonical serialization
(`canonicalEventJson`) is byte-identical to the Python mirror
(`spendguard.integrations.ag_ui`), proven against the shared fixture corpus
`sdk/fixtures/cross-language/ag_ui_v1.json`.

## API

- `SPENDGUARD_AG_UI_EVENT_NAMES` — the five-name vocabulary constant.
- `buildBudgetSnapshot` / `buildReservationCreated` /
  `buildReservationCommitted` / `buildReservationReleased` /
  `buildDecisionDenied` — pure event builders.
- `canonicalEventJson(event)` — locked canonical JSON (sorted keys, UTF-8,
  no whitespace, ASCII-only keys, integer-only numbers).
- `encodeSse(event)` — `"data: " + canonicalEventJson(event) + "\n\n"`.
- `AgUiEventValidationError` — thrown on invalid builder input; `.field`
  names the payload key.
- `VERSION` — package version constant.

Optional string fields (`unitId`, `runId`, `llmCallId`, …) treat the empty
string and absent as the same thing: the payload key is **omitted**. An empty
`unit_id` is never emitted.

## Why `@ag-ui/core` is optional (0.x churn isolation)

AG-UI is entirely 0.x with a fast release cadence and no spec-stability
policy. This package owns its event types — they structurally match the AG-UI
`CUSTOM` event shape (`{type: "CUSTOM", name, value, timestamp?}`) and are
compat-tested against an exactly pinned `@ag-ui/core` in CI. If an AG-UI
release ever moves, our wire shape does not: consumers key off
`schema_version` in each payload. You never need AG-UI packages installed to
use the builders.

## License

Apache-2.0 — see the repository root
[LICENSE](https://github.com/m24927605/agentic-spendguard/blob/main/LICENSE)
and [`LICENSE_NOTICES.md`](./LICENSE_NOTICES.md).
