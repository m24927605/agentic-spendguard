# COV_D41S_06 - Sidecar-to-ledger session bridge

> **Deliverable:** D41 sidecar-to-ledger session bridge
> **Slice:** 6 follow-up after D41 session reservation substrate
> **Spec set:** [`docs/specs/coverage/D41_sidecar_session_bridge/`](../../specs/coverage/D41_sidecar_session_bridge/)
> **Substrate spec:** [`docs/specs/coverage/D41_session_reservation_substrate/`](../../specs/coverage/D41_session_reservation_substrate/)
> **Precedence:** bridge and substrate `design.md` files are LOCKED and trump this doc.

## Scope

Replace the fail-closed sidecar UDS session stubs with an explicit
sidecar-to-ledger runtime bridge. Add internal Ledger gRPC session RPCs,
Ledger handlers that call the shipped SQL substrate, sidecar mapping code,
focused tests, and a sidecar-path demo gate.

## LOCKED design quotes

From `D41_sidecar_session_bridge/design.md` §4:

> The sidecar must not import `sqlx` or `spendguard-ledger` to shortcut this
> path. Ledger remains the only process that talks to the ledger database.

From `D41_sidecar_session_bridge/design.md` §6:

> Ledger handlers own the server-minted session audit envelope.

From `D41_session_reservation_substrate/design.md` §12 closeout correction:

> A follow-up bridge slice must wire the sidecar RPC handlers to an explicit
> ledger session API before LiveKit/Pipecat adapters call these methods.

## Files touched

| File | Why |
|---|---|
| `proto/spendguard/ledger/v1/ledger.proto` | Add internal Ledger session RPCs/messages. |
| generated Rust proto artifacts | Ledger and sidecar generated client/server code. |
| `services/ledger/src/handlers/session_reservations.rs` | New Ledger gRPC handlers over existing SQL wrappers. |
| `services/ledger/src/handlers/mod.rs` | Register handler module. |
| `services/ledger/src/server.rs` | Implement new Ledger trait methods and metrics recording. |
| `services/ledger/src/metrics.rs` | Add handler buckets if needed. |
| `services/sidecar/src/clients/ledger.rs` | Add Ledger client methods. |
| `services/sidecar/src/session_bridge.rs` | New sidecar mapping/validation module. |
| `services/sidecar/src/server/adapter_uds.rs` | Replace `UNIMPLEMENTED` stubs with bridge calls. |
| `services/sidecar/src/metrics.rs` | Keep session handler buckets accurate. |
| `services/ledger/tests/session_bridge.rs` | Ledger gRPC bridge tests. |
| `services/sidecar/tests/session_bridge_test.rs` | Sidecar bridge tests. |
| `deploy/demo/session_bridge/*` | Sidecar-path demo overlay and driver. |
| `deploy/demo/verify_step_session_bridge*.sql` | Ledger and canonical SQL hard gates. |
| `deploy/demo/Makefile` | `DEMO_MODE=session_bridge` and verify target. |

## VERIFY-AT-IMPL pins

Pin `SB-V1`..`SB-V5` from bridge `design.md` §10.

## Test/verification plan

- TP-D41B-01..08.
- TP-D41B-20..28.
- TA-D41B-01..06.

Required commands:

```bash
cargo test --manifest-path services/ledger/Cargo.toml session_bridge
cargo test --manifest-path services/sidecar/Cargo.toml session_bridge
make demo-down
make demo-up DEMO_MODE=session_bridge
make -C deploy/demo demo-verify-session-bridge
make demo-down
make demo-up DEMO_MODE=session_reservation
make -C deploy/demo demo-verify-session-reservation
```

Closeout also reruns the D38/D39 sidecar request-scoped demos listed in
bridge `acceptance.md` §4.

## Anti-scope

- No LiveKit/Pipecat adapter behavior.
- No adapter-facing session proto changes.
- No sidecar direct Postgres connection.
- No local per-request fallback for voice sessions.
- No edits to frozen cross-language fixtures.
