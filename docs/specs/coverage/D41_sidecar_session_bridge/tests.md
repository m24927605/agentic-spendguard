# D41 sidecar-to-ledger session bridge - Tests

## 1. Ledger gRPC tests

| ID | Test | Verifies |
|---|---|---|
| TP-D41B-01 | Ledger proto codegen exposes `ReserveSession`, `CommitSessionDelta`, and `ReleaseSession`. | `SB-V1`. |
| TP-D41B-02 | `ReserveSessionLedger` accepted branch creates session hold and signed reserve audit pair. | Ledger handler calls existing SQL and builds audit context. |
| TP-D41B-03 | `ReserveSessionLedger` denied branch returns typed denied response and signed denied audit pair. | No gRPC transport failure for business DENY. |
| TP-D41B-04 | `CommitSessionDeltaLedger` commits a positive delta and replay does not double count. | Streaming commit idempotency. |
| TP-D41B-05 | Same `streaming_commit_id` with different amount returns idempotency conflict. | Replay safety. |
| TP-D41B-06 | Over-reserve commit returns typed error and does not change committed/released balances. | Hard cap. |
| TP-D41B-07 | Tuple mismatch on commit returns typed error. | Unit/window/pricing integrity. |
| TP-D41B-08 | `ReleaseSessionLedger` releases only uncommitted remainder and replay is idempotent. | Settlement. |

## 2. Sidecar bridge tests

| ID | Test | Verifies |
|---|---|---|
| TP-D41B-20 | Sidecar `ReserveSession` maps valid adapter request to Ledger request with exact tuple fields. | `SB-V3` mapping. |
| TP-D41B-21 | Missing `unit.unit_id`, missing pricing fields, zero amount, or missing event time is rejected before Ledger call. | Fail-fast validation. |
| TP-D41B-22 | Accepted Ledger reserve maps to `ReserveSessionOutcome.accepted`. | Adapter runtime success path. |
| TP-D41B-23 | Denied Ledger reserve maps to `ReserveSessionOutcome.denied` and not gRPC `UNAVAILABLE`. | Typed fail-closed DENY. |
| TP-D41B-24 | Ledger transport failure maps to gRPC `UNAVAILABLE`. | Adapter fail-closed outage path. |
| TP-D41B-25 | Commit accepted/replay maps to `CommitSessionDeltaOutcome.accepted`. | Streaming commit success path. |
| TP-D41B-26 | Commit over-budget/conflict maps to `CommitSessionDeltaOutcome.error`. | No silent continuation. |
| TP-D41B-27 | Release accepted/replay maps to `ReleaseSessionOutcome.accepted`. | Release path. |
| TP-D41B-28 | Sidecar metrics buckets include reserve/commit/release session handlers. | Observability. |

## 3. Demo and acceptance tests

| ID | Command | Pass condition |
|---|---|---|
| TA-D41B-01 | `cargo test --manifest-path services/ledger/Cargo.toml session_bridge` | Ledger gRPC session bridge focused tests pass. |
| TA-D41B-02 | `cargo test --manifest-path services/sidecar/Cargo.toml session_bridge` | Sidecar bridge focused tests pass. |
| TA-D41B-03 | `make demo-down` | exits 0. |
| TA-D41B-04 | `make demo-up DEMO_MODE=session_bridge` | prints locked success line. |
| TA-D41B-05 | `make -C deploy/demo demo-verify-session-bridge` | ledger and canonical SQL hard gates pass. |
| TA-D41B-06 | `make demo-up DEMO_MODE=session_reservation` and `make -C deploy/demo demo-verify-session-reservation` | existing direct substrate runner and SQL hard gates still pass. |

## 4. Slice mapping

| Slice | Tests |
|---|---|
| `COV_D41S_06_sidecar_session_bridge` | TP-D41B-01..08, TP-D41B-20..28, TA-D41B-01..06. |
