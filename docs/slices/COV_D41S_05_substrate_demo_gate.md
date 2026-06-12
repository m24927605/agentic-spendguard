# COV_D41S_05 - Session reservation demo and substrate closeout

> **Deliverable:** D41 session reservation substrate
> **Slice:** 5 of 5
> **Spec set:** [`docs/specs/coverage/D41_session_reservation_substrate/`](../specs/coverage/D41_session_reservation_substrate/)
> **Precedence:** `design.md` is LOCKED and trumps this doc.

## Scope

Add local `session_reservation` demo mode, hard verify SQL, audit/canonical ingest proof, request-scoped non-regression gates, and closeout docs needed before D41 voice adapters can ship.

## LOCKED design quotes

From `implementation.md` §5:

> Locked success line:
>
> `[demo] session_reservation ALL 7 steps PASS`

From `acceptance.md` §6:

> D41 adapter docs reference this substrate instead of inventing local lifecycle rules.

## Files touched

| File | Why |
|---|---|
| `deploy/demo/session_reservation/*` | Demo overlay and driver. |
| `deploy/demo/verify_step_session_reservation.sql` | Hard SQL gate. |
| `deploy/demo/verify_step_session_reservation_canonical.sql` | Canonical DB hard gate scoped to the three demo session reservation IDs. |
| `deploy/demo/Makefile` | Demo branch and verify target. |
| `services/ledger/src/session_reservations.rs` | Signed audit context/server mint request threading for focused ledger entrypoints. |
| `services/ledger/tests/session_reservations.rs` | Regression coverage for required signed-shape session audit context. |
| docs/specs or site docs | Handoff note to D41 adapter docs if needed. |

## VERIFY-AT-IMPL pins

Pin `SR-V5`; confirm all `SR-V*` markers are closed.

## Test/verification plan

- TA-D41S-01..07.
- Request-scoped non-regression demos from acceptance.md §5.

## Anti-scope

- No LiveKit/Pipecat adapter behavior.
- No dashboard UI.
