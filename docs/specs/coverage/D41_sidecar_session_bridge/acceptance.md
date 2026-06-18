# D41 sidecar-to-ledger session bridge - Acceptance Gates

## 1. Spec and proto

| Gate | Command | Pass condition |
|---|---|---|
| A1.1 | `rg -n "rpc ReserveSession|rpc CommitSessionDelta|rpc ReleaseSession" proto/spendguard/ledger/v1/ledger.proto` | Ledger service exposes the three internal session RPCs. |
| A1.2 | `cargo test --manifest-path services/ledger/Cargo.toml session_bridge` | Ledger proto/handler focused tests pass. |
| A1.3 | `cargo test --manifest-path services/sidecar/Cargo.toml session_bridge` | Sidecar generated client and bridge focused tests pass. |

## 2. Runtime mapping

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | Sidecar focused test for reserve accepted | `ReserveSessionOutcome.accepted` returned and Ledger received exact tuple. |
| A2.2 | Sidecar focused test for reserve denied | `ReserveSessionOutcome.denied` returned; provider-facing callers can fail closed with DENY metadata. |
| A2.3 | Sidecar focused test for Ledger transport failure | gRPC `UNAVAILABLE`; no fake accepted/denied outcome. |
| A2.4 | Sidecar focused test for commit over-budget/conflict | `CommitSessionDeltaOutcome.error`; further provider turns must stop. |
| A2.5 | Sidecar focused test for release replay | original release outcome returned without fresh mutation. |

## 3. Demo

| Gate | Command | Pass condition |
|---|---|---|
| A3.1 | `make demo-down` | exit 0. |
| A3.2 | `make demo-up DEMO_MODE=session_bridge` | prints `[demo] session_bridge ALL 5 steps PASS (RESERVE + COMMIT + REPLAY + DENY + RELEASE)`. |
| A3.3 | `make -C deploy/demo demo-verify-session-bridge` | ledger and canonical SQL hard gates pass. |
| A3.4 | `make demo-down` | exit 0 after the bridge demo. |

## 4. Non-regression

| Gate | Command | Pass condition |
|---|---|---|
| A4.1 | `make demo-up DEMO_MODE=session_reservation` | existing D41S direct-ledger substrate demo runner still passes. |
| A4.2 | `make -C deploy/demo demo-verify-session-reservation` | existing D41S direct-ledger ledger/canonical SQL hard gates still pass. |
| A4.3 | `make demo-up DEMO_MODE=ag_ui_events` | D39 sidecar request-scoped path still passes. |
| A4.4 | `make demo-up DEMO_MODE=mastra_processor` | D38 sidecar request-scoped path still passes. |

## 5. Ship checklist

- [ ] `SB-V1`..`SB-V5` pinned in the slice doc or dated design amendment.
- [ ] Sidecar session RPC handlers no longer return `UNIMPLEMENTED`.
- [ ] Ledger handler owns session audit context and Ed25519 signatures.
- [ ] Sidecar does not connect directly to Postgres.
- [ ] Existing direct `session_reservation` demo remains green.
- [ ] D41 voice adapter work is still blocked unless this bridge is on main.
