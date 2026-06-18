# COV_S05_04 — D05 TS SDK substrate: handshake + reserve + commitEstimated

> **Deliverable**: D05 TS SDK substrate
> **Slice**: 4 of 10 (M)
> **Spec set**: [`docs/specs/coverage/D05_ts_sdk_substrate/`](../../specs/coverage/D05_ts_sdk_substrate/)

## Scope

Replace SLICE 3's `throw new SpendGuardError(...wired in SLICE 4-5)` stubs with real gRPC bodies for `handshake()`, `reserve()` (which is `requestDecision`), and `commitEstimated()` (single-event LLM_CALL_POST path).

Concretely:
- `sdk/typescript/src/client.ts`:
  - **`handshake()`**: real `this.adapterClient!.handshake({...})` call. On success: store sessionId for subsequent reserves. Idempotent (per design §4.5).
  - **`reserve(req: ReserveRequest)`**: real `this.adapterClient!.requestDecision({...})` call. Maps `ReserveRequest` → proto `RequestDecisionRequest`; maps `RequestDecisionResponse` → `ReserveResponse`. On DecisionDenied: throw `SpendGuardDecisionError` with reason_codes.
  - **`requestDecision = this.reserve.bind(this)`** — instance-field initializer making identity Boolean-true (review-standards §1.5 P0 blocker)
  - **`commitEstimated(req)`**: real `this.adapterClient!.emitTraceEvents({...})` call with single LLM_CALL_POST event.
  - `runProjectionDefault` consumed in `buildDecisionRequest` per implementation.md §4.
  - Disabled-mode short-circuits added per implementation.md §4 (`if (this.cfg.disabled) return makeDisabledX(req)` helpers).
- `sdk/typescript/src/config.ts`:
  - Add `export type RunProjectionPolicy = "STRICT_CEILING" | "ELASTIC" | (string & {})` per design §4.2 R2 amendment (closes MJ-1 from SLICE 3 R2 review)
  - Retype `runProjectionDefault?: RunProjectionPolicy` on `SpendGuardClientConfig` (was `?: string`)
  - Re-export `RunProjectionPolicy` from `./index` AND `./client` subpaths
- `sdk/typescript/tests/`:
  - Extend `_support/mockSidecar.ts` to register `SidecarAdapter` service with handshake / requestDecision / emitTraceEvents handlers that return shaped responses.
  - New tests: handshake idempotency, reserve ALLOW/DENY/DEGRADE outcomes, commitEstimated success, requestDecision === reserve identity assertion, disabled-mode short-circuits.
  - Extend locked-surface.test.ts with `RunProjectionPolicy` field-type assertion.

## Files touched

| File | Why |
|------|-----|
| `sdk/typescript/src/client.ts` | Real RPC bodies for handshake / reserve / commitEstimated |
| `sdk/typescript/src/config.ts` | RunProjectionPolicy type + retype runProjectionDefault |
| `sdk/typescript/src/index.ts` | Re-export RunProjectionPolicy |
| `sdk/typescript/tests/_support/mockSidecar.ts` | Real service handlers |
| `sdk/typescript/tests/client.test.ts` | New RPC body tests |
| `sdk/typescript/tests/locked-surface.test.ts` | RunProjectionPolicy field assertion |

## Test/verification plan

1. `pnpm run typecheck` clean.
2. `pnpm run test` — 107 + ~15 new = ~122 passing.
3. `pnpm run build` clean; dist surfaces stable.
4. `pnpm run lint` clean.
5. Key new tests:
   - `requestDecision === reserve` identity (review-standards §1.5 P0)
   - `RunProjectionPolicy` field exposed at type level (MJ-1 closure)
   - Handshake idempotency: calling twice with same params reuses sessionId
   - Reserve DENY raises SpendGuardDecisionError with reason_codes
   - commitEstimated single-event success path

## Anti-scope

- No `release()` body — SLICE 05.
- No `queryBudget()` body — SLICE 05.
- No multi-event commit (commit + outcome) — SLICE 05.
- No `ids.ts` / `promptHash.ts` / `pricing.ts` — SLICE 06.
- No `withRunPlan` — SLICE 07.
- No OTel / retry / idempotency cache — SLICE 08.

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D05_ts_sdk_substrate/design.md) §4.2 LOCKED options, §4.5 lifecycle, §4.7 reserve, §4.8 commitEstimated
- SLICE 3 R2 commitments: `requestDecision === reserve` (§1.5), `RunProjectionPolicy` type (MJ-1), ledger GH issue
- SLICE 3: [`COV_S05_03_d05_client_skeleton.md`](COV_S05_03_d05_client_skeleton.md)
