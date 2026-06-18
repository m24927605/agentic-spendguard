# D41 session reservation substrate - Implementation

## 1. File layout

Exact paths may change if `SR-V1` selects a different proto package, but the implementation must keep this ownership split:

```text
proto/spendguard/sidecar_adapter/v1/adapter.proto
services/ledger/
  migrations/<next>_session_reservations.sql
  src/session_reservations.rs
services/sidecar/
  src/session_reservations.rs
  src/server/adapter_uds.rs
sdk/typescript/src/session.ts
sdk/typescript/tests/session-reservation.test.ts
sdk/python/src/spendguard/session.py
sdk/python/tests/test_session_reservation.py
deploy/demo/session_reservation/
  docker-compose.yaml
  driver.py
deploy/demo/verify_step_session_reservation.sql
```

## 2. Storage model

Minimum logical tables:

| Table | Purpose |
|---|---|
| `session_reservations` | One row per live session hold; stores tuple, reserved, committed, released, status. |
| `session_commit_deltas` | Idempotent delta ledger; one row per streaming commit id. |
| `session_reservation_events` | Optional narrow event projection if existing audit outbox is insufficient. |

The ledger transaction must guarantee:

```text
0 <= committed_atomic <= reserved_atomic
released_atomic = reserved_atomic - committed_atomic at final release
```

Any commit that would exceed reserved amount is rejected and emits a denial/error audit event.

## 3. SDK surfaces

TypeScript conceptual surface:

```ts
interface ReserveSessionRequest { /* design.md §5 fields */ }
interface CommitSessionDeltaRequest { /* design.md §5 fields */ }
interface ReleaseSessionRequest { /* design.md §5 fields */ }

class SpendGuardClient {
  reserveSession(req: ReserveSessionRequest): Promise<ReserveSessionOutcome>;
  commitSessionDelta(req: CommitSessionDeltaRequest): Promise<CommitSessionDeltaOutcome>;
  releaseSession(req: ReleaseSessionRequest): Promise<ReleaseSessionOutcome>;
}
```

Python conceptual surface:

```python
async def reserve_session(self, req: ReserveSessionRequest) -> ReserveSessionOutcome: ...
async def commit_session_delta(self, req: CommitSessionDeltaRequest) -> CommitSessionDeltaOutcome: ...
async def release_session(self, req: ReleaseSessionRequest) -> ReleaseSessionOutcome: ...
```

Exact generated type names are pinned by `SR-V3`.

## 4. Retry and reconnect

The SDK may provide a helper `SessionReservationHandle` that tracks:

- `session_reservation_id`
- next monotonic `streaming_commit_id`
- pending deltas not yet acked
- release status

The helper must not hide semantic errors. Idempotency conflicts, over-budget commits, and tuple mismatches surface as typed errors.

## 5. Demo driver

`deploy/demo/session_reservation/driver.py` performs:

1. Reserve session for 100000 atomic units.
2. Commit delta 1000.
3. Commit delta 2000.
4. Replay delta 2000 with same id and same payload; expect idempotent success/no double count.
5. Try conflicting replay with same id and different amount; expect conflict.
6. Try over-reserve delta; expect rejection.
7. Release remainder.

Locked success line:

```text
[demo] session_reservation ALL 7 steps PASS
```

## 6. Backward compatibility

Existing request-scoped `reserve`, `commitEstimated`, and `release` APIs must remain unchanged. All existing adapter demos are non-regression gates at substrate closeout; D41 substrate may not break D38/D39 or D04/D06/D08/D29 TS adapter demos.

## 7. Sidecar bridge follow-up

The substrate closeout intentionally left the sidecar adapter UDS session RPCs
as fail-closed `UNIMPLEMENTED` stubs. Runtime adapter coverage requires the
follow-up bridge spec:

```text
docs/specs/coverage/D41_sidecar_session_bridge/
docs/internal/slices/COV_D41S_06_sidecar_session_bridge.md
```

Implementation order is locked:

1. Keep this direct-ledger substrate demo green.
2. Ship `COV_D41S_06_sidecar_session_bridge`.
3. Only then implement D41 LiveKit/Pipecat adapters.
