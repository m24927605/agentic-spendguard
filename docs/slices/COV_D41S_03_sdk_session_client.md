# COV_D41S_03 - SDK session client surfaces

> **Deliverable:** D41 session reservation substrate
> **Slice:** 3 of 5
> **Spec set:** [`docs/specs/coverage/D41_session_reservation_substrate/`](../specs/coverage/D41_session_reservation_substrate/)
> **Precedence:** `design.md` is LOCKED and trumps this doc.

## Scope

Expose TS and Python SDK methods for reserve session, commit session delta, and release session. Add focused tests and import documentation.

## LOCKED design quotes

From `implementation.md` §3:

> `reserveSession(req: ReserveSessionRequest): Promise<ReserveSessionOutcome>;`
>
> `commitSessionDelta(req: CommitSessionDeltaRequest): Promise<CommitSessionDeltaOutcome>;`
>
> `releaseSession(req: ReleaseSessionRequest): Promise<ReleaseSessionOutcome>;`

From `design.md` §6:

> Replay with byte-identical payload returns the original outcome. Replay with same key and different payload returns idempotency conflict.

## Files touched

| File | Why |
|---|---|
| `sdk/typescript/src/session.ts` | TS request/outcome types and helpers. |
| `sdk/typescript/src/client.ts` | Client methods. |
| `sdk/typescript/tests/session-reservation.test.ts` | TS tests. |
| `sdk/python/src/spendguard/session.py` | Python types/helpers. |
| `sdk/python/src/spendguard/client.py` | Python client methods. |
| `sdk/python/tests/test_session_reservation.py` | Python tests. |

## VERIFY-AT-IMPL pins

Pin `SR-V3`.

## Test/verification plan

- TP-D41S-10..13.
- A3.1..A3.3.

## Anti-scope

- No framework adapter code.
- No demo closeout.
