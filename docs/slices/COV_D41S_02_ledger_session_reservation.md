# COV_D41S_02 - Ledger session reservation semantics

> **Deliverable:** D41 session reservation substrate
> **Slice:** 2 of 5
> **Spec set:** [`docs/specs/coverage/D41_session_reservation_substrate/`](../specs/coverage/D41_session_reservation_substrate/)
> **Precedence:** `design.md` is LOCKED and trumps this doc.

## Scope

Implement ledger storage, transaction boundaries, reserve/commit/release procedures, and invariants for session reservations.

## LOCKED design quotes

From `design.md` §4:

> The session reservation is a ledger hold, not a credit line. Every commit reduces the held remainder and increases committed spend. Release settles only the uncommitted remainder.

From `implementation.md` §2:

> `0 <= committed_atomic <= reserved_atomic`
>
> `released_atomic = reserved_atomic - committed_atomic at final release`

## Files touched

| File | Why |
|---|---|
| `services/ledger/migrations/<next>_session_reservations.sql` | Tables/indexes/functions. |
| `services/ledger/src/session_reservations.rs` | Ledger logic. |
| `services/sidecar/src/session_reservations.rs` | Sidecar transaction adapter if needed. |
| ledger/sidecar tests | TP-D41S-01..09, 14. |

## VERIFY-AT-IMPL pins

Pin `SR-V2`.

## Test/verification plan

- TP-D41S-01..09 and TP-D41S-14.
- A2.1..A2.4.

## Anti-scope

- No SDK helper ergonomics beyond generated usage.
- No voice adapters.
