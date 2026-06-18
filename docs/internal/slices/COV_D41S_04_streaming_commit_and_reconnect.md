# COV_D41S_04 - Streaming commit retry and reconnect

> **Deliverable:** D41 session reservation substrate
> **Slice:** 4 of 5
> **Spec set:** [`docs/specs/coverage/D41_session_reservation_substrate/`](../../specs/coverage/D41_session_reservation_substrate/)
> **Precedence:** `design.md` is LOCKED and trumps this doc.

## Scope

Implement bounded pending-delta retry/replay, reconnect behavior, idempotency conflict handling, and TTL sweep integration.

## LOCKED design quotes

From `design.md` §8:

> Reconnect after network drop - Reuse same `session_reservation_id`; replay already-sent deltas by `streaming_commit_id`.

From `design.md` §8:

> Process crash - TTL sweep releases uncommitted remainder.

## Files touched

| File | Why |
|---|---|
| SDK session handle files | Pending-delta buffer and replay. |
| sidecar/ledger session files | TTL and conflict handling. |
| focused reconnect tests | Replay and bounded retry tests. |

## VERIFY-AT-IMPL pins

Pin `SR-V4`.

## Test/verification plan

- Reconnect replay tests.
- TP-D41S-02..09.
- TTL sweep tests.

## Anti-scope

- No voice adapter code.
- No unbounded local buffering.
