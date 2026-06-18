# COV_D41S_01 - Session reservation contract and proto

> **Deliverable:** D41 session reservation substrate
> **Slice:** 1 of 5
> **Spec set:** [`docs/specs/coverage/D41_session_reservation_substrate/`](../../specs/coverage/D41_session_reservation_substrate/)
> **Precedence:** `design.md` is LOCKED and trumps this doc.

## Scope

Add the proto/API contract for session reservations and generate language bindings. This slice pins field numbers and service placement, but does not implement ledger semantics.

## LOCKED design quotes

From `design.md` §5:

> `rpc ReserveSession(ReserveSessionRequest) returns (ReserveSessionOutcome)`
>
> `rpc CommitSessionDelta(CommitSessionDeltaRequest) returns (CommitSessionDeltaOutcome)`
>
> `rpc ReleaseSession(ReleaseSessionRequest) returns (ReleaseSessionOutcome)`

From `design.md` §5:

> Every `amount_atomic_delta` must be a positive decimal string. Zero commits are rejected.

## Files touched

| File | Why |
|---|---|
| `proto/spendguard/sidecar_adapter/v1/adapter.proto` | Add session RPC/messages or amended location. |
| generated TS/Python/Rust proto files | Generated bindings. |
| SDK skeleton files | Type placeholders only. |
| focused proto tests | Wire construction skeleton. |

## VERIFY-AT-IMPL pins

Pin `SR-V1`.

## Test/verification plan

- A1.1 proto generation.
- TP-D41S-10..13 skeleton where applicable.

## Anti-scope

- No ledger stored procedures.
- No demo.
- No voice framework adapters.
