# D41 sidecar-to-ledger session bridge - Implementation

## 1. File layout

```text
proto/spendguard/ledger/v1/ledger.proto
services/ledger/src/server.rs
services/ledger/src/metrics.rs
services/ledger/src/handlers/mod.rs
services/ledger/src/handlers/session_reservations.rs
services/ledger/src/session_reservations.rs
services/ledger/tests/session_bridge.rs
services/sidecar/src/clients/ledger.rs
services/sidecar/src/server/adapter_uds.rs
services/sidecar/src/session_bridge.rs
services/sidecar/src/metrics.rs
services/sidecar/tests/session_bridge_test.rs
deploy/demo/session_bridge/
  docker-compose.yaml
  driver.py
deploy/demo/verify_step_session_bridge.sql
deploy/demo/verify_step_session_bridge_canonical.sql
```

If implementation reuses `services/ledger/tests/session_reservations.rs`
instead of adding `session_bridge.rs`, the slice doc must record that choice
with a dated pin.

## 2. Ledger proto delta

Add three internal Ledger RPCs to `proto/spendguard/ledger/v1/ledger.proto`.
The adapter-facing sidecar proto is not changed.

```protobuf
rpc ReserveSession(ReserveSessionLedgerRequest)
    returns (ReserveSessionLedgerResponse);
rpc CommitSessionDelta(CommitSessionDeltaLedgerRequest)
    returns (CommitSessionDeltaLedgerResponse);
rpc ReleaseSession(ReleaseSessionLedgerRequest)
    returns (ReleaseSessionLedgerResponse);
```

`ReserveSessionLedgerRequest` carries the SR-V1 reserve fields plus the
internal scalar tuple fields used by `ReserveSessionLedgerRequest` in
`services/ledger/src/session_reservations.rs`.

`CommitSessionDeltaLedgerRequest` carries the SR-V1 commit fields plus optional
defense-in-depth tuple fields. If present, the SQL substrate enforces tuple
equality with the reservation row.

`ReleaseSessionLedgerRequest` carries the SR-V1 release fields.

## 3. Ledger handler

`services/ledger/src/handlers/session_reservations.rs` should:

1. Validate proto request fields before building JSON.
2. Mint `session_reservation_id` and `ttl_expires_at` for reserve.
3. Build signed `audit_context` using `LedgerService.signer`.
4. Call the existing Rust wrappers:
   - `session_reservations::reserve_session`
   - `session_reservations::commit_session_delta`
   - `session_reservations::release_session`
5. Convert JSON outcomes into typed Ledger proto responses.

Do not duplicate SQL-side idempotency or balance logic in the handler.

## 4. Sidecar bridge

`services/sidecar/src/session_bridge.rs` should be the only new sidecar module
that understands D41 session outcome mapping. It should expose small functions
called by `adapter_uds.rs`:

```rust
pub async fn reserve_session(
    state: &SidecarState,
    req: ReserveSessionRequest,
) -> Result<ReserveSessionOutcome, tonic::Status>;

pub async fn commit_session_delta(
    state: &SidecarState,
    req: CommitSessionDeltaRequest,
) -> Result<CommitSessionDeltaOutcome, tonic::Status>;

pub async fn release_session(
    state: &SidecarState,
    req: ReleaseSessionRequest,
) -> Result<ReleaseSessionOutcome, tonic::Status>;
```

The bridge validates adapter wire shape, maps to Ledger proto requests, calls
`state.inner.ledger`, and maps Ledger responses back to adapter proto outcomes.

## 5. Demo

Add `DEMO_MODE=session_bridge`. The demo must call the sidecar UDS RPCs, not
`post_session_*` SQL functions directly.

Locked success line:

```text
[demo] session_bridge ALL 5 steps PASS (RESERVE + COMMIT + REPLAY + DENY + RELEASE)
```

Minimum scenario:

1. Reserve an accepted session through sidecar UDS.
2. Commit one positive delta through sidecar UDS.
3. Replay the same delta and prove no double count.
4. Reserve a denied session and prove the result is `ReserveSessionDenied`.
5. Release the accepted session and prove the remainder is settled.

The SQL hard gate must also prove canonical ingest received the session audit
decision/outcome pairs emitted by the Ledger handler.

## 6. Backward compatibility

The direct `DEMO_MODE=session_reservation` substrate demo remains valid and
must keep passing. It proves the ledger SQL substrate independently of the
sidecar bridge. The new `session_bridge` demo proves the runtime path adapters
will use.
