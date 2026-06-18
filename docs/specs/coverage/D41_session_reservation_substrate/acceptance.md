# D41 session reservation substrate - Acceptance Gates

## 1. Contract and migration

| Gate | Command | Pass condition |
|---|---|---|
| A1.1 | proto generation command used by repo | generated TS/Python/Rust artifacts update cleanly. |
| A1.2 | migration smoke command used by repo | session reservation migration applies to empty DB and upgraded DB. |
| A1.3 | `rg -n "ReserveSession|CommitSessionDelta|ReleaseSession" proto services sdk` | semantic API names present or dated amendment explains replacement. |

## 2. Ledger and sidecar

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | Rust ledger/sidecar focused tests | exit 0. |
| A2.2 | idempotency conflict focused tests | same-key/different-payload conflicts pass. |
| A2.3 | over-budget commit focused test | commit delta beyond reservation rejected. |
| A2.4 | TTL sweep focused test | expired session releases remainder. |

## 3. SDK

| Gate | Command | Pass condition |
|---|---|---|
| A3.1 | TS SDK session tests | exit 0. |
| A3.2 | Python SDK session tests | exit 0. |
| A3.3 | cross-language fixture verifier, if a fixture is added | byte-equivalent. |

## 4. Demo

| Gate | Command | Pass condition |
|---|---|---|
| A4.1 | `make demo-down` | exit 0. |
| A4.2 | `make demo-up DEMO_MODE=session_reservation` | prints `[demo] session_reservation ALL 7 steps PASS`. |
| A4.3 | `make -C deploy/demo demo-verify-session-reservation` | SQL hard gate exits 0. |

## 5. Non-regression

At closeout, rerun at least:

- `make demo-up DEMO_MODE=mastra_processor`
- `make demo-up DEMO_MODE=ag_ui_events`
- the current TS SDK test command
- the current Python SDK focused test command

Any failure caused by D41 substrate blocks closeout.

## 6. Ship checklist

- [ ] `SR-V1`..`SR-V5` pinned.
- [ ] Session demo physically run after `make demo-down`.
- [ ] Existing request-scoped lifecycle unchanged.
- [ ] D41 adapter docs reference this substrate instead of inventing local lifecycle rules.

## 7. Follow-up bridge prerequisite

The substrate ship checklist above is not enough for LiveKit/Pipecat runtime
adapter work. Before any adapter calls `ReserveSession`,
`CommitSessionDelta`, or `ReleaseSession`, ship:

```text
docs/specs/coverage/D41_sidecar_session_bridge/
docs/internal/slices/COV_D41S_06_sidecar_session_bridge.md
```

Minimum bridge gates:

| Gate | Command | Pass condition |
|---|---|---|
| B1 | `cargo test --manifest-path services/ledger/Cargo.toml session_bridge` | Ledger session gRPC tests pass. |
| B2 | `cargo test --manifest-path services/sidecar/Cargo.toml session_bridge` | Sidecar bridge tests pass. |
| B3 | `make demo-down` | exit 0. |
| B4 | `make demo-up DEMO_MODE=session_bridge` | prints `[demo] session_bridge ALL 5 steps PASS (RESERVE + COMMIT + REPLAY + DENY + RELEASE)`. |
| B5 | `make -C deploy/demo demo-verify-session-bridge` | sidecar-path ledger/canonical SQL gates pass. |
