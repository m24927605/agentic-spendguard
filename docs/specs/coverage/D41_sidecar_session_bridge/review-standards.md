# D41 sidecar-to-ledger session bridge - Review Standards

Use with an independent Codex sub-agent reviewer for every bridge slice.

## 1. Precedence (P0)

`D41_sidecar_session_bridge/design.md` owns bridge behavior. The existing
`D41_session_reservation_substrate/design.md` owns ledger semantics. The
bridge may not weaken either document.

## 2. Architecture invariants (P0)

| Check | Pass condition |
|---|---|
| 2.1 | Sidecar talks to Ledger over the existing mTLS gRPC client; no direct DB path. |
| 2.2 | Adapter-facing sidecar session proto remains SR-V1 compatible. |
| 2.3 | Ledger handler calls the existing session SQL wrappers; no duplicate balance/idempotency implementation. |
| 2.4 | Ledger handler, not adapter code, owns session audit context and signatures. |

## 3. Failure posture (P0)

| Check | Pass condition |
|---|---|
| 3.1 | Ledger transport failure maps to gRPC `UNAVAILABLE` and fails closed. |
| 3.2 | Reserve DENY maps to typed `ReserveSessionDenied`, not a transport error. |
| 3.3 | Commit over-budget/conflict maps to an error outcome and cannot silently continue. |
| 3.4 | Missing unit/window/pricing/event-time fields are rejected before any ledger mutation. |

## 4. Ledger invariants (P0)

| Check | Pass condition |
|---|---|
| 4.1 | Positive commit deltas only. |
| 4.2 | Commit cannot exceed reserved amount. |
| 4.3 | Same idempotency key with different payload conflicts. |
| 4.4 | Release settles only uncommitted remainder. |
| 4.5 | Signed session audit rows reach canonical ingest in the demo. |

## 5. Backward compatibility (P1)

| Check | Pass condition |
|---|---|
| 5.1 | Request-scoped `RequestDecision`, `EmitTraceEvents`, and `ReleaseReservation` paths are unchanged. |
| 5.2 | Existing `session_reservation` direct-ledger demo still passes. |
| 5.3 | D38/D39 demos still pass at closeout. |

## 6. Reviewer prompt template

```text
You are the adversarial code reviewer for slice COV_D41S_06_sidecar_session_bridge
(round R<N>) of D41 sidecar-to-ledger session bridge.

Reviewer tool: independent Codex sub-agent.

Read in order:
1. docs/specs/coverage/D41_session_reservation_substrate/design.md
2. docs/specs/coverage/D41_sidecar_session_bridge/design.md
3. docs/specs/coverage/D41_sidecar_session_bridge/review-standards.md
4. docs/internal/slices/COV_D41S_06_sidecar_session_bridge.md
5. The diff under review: <DIFF_REF>

Block on direct sidecar DB access, adapter-facing proto churn, fake session
audit signatures, weakened idempotency, fail-open outage handling, missing
unit/window/pricing validation, or demos that bypass sidecar UDS.

Output numbered findings with severity, file:line, evidence, and spec ref.
End with exactly one verdict line:
VERDICT: PASS
or
VERDICT: FAIL (<b> blockers, <m> majors)
```
