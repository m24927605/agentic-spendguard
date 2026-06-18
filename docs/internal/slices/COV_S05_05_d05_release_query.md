# COV_S05_05 — D05 TS SDK substrate: release + queryBudget + multi-event commit

> **Deliverable**: D05 TS SDK substrate
> **Slice**: 5 of 10 (M)
> **Spec set**: [`docs/specs/coverage/D05_ts_sdk_substrate/`](../../specs/coverage/D05_ts_sdk_substrate/)

## Scope

Wire the three remaining hot-path RPCs that SLICE 4 left as `SLICE_5_NOT_WIRED` stubs:
1. `release()` — real `releaseReservation` RPC (ASP Draft-01 §4 one-to-one)
2. `queryBudget()` — placeholder per design §9.4 (sidecar RPC not yet shipped; TS surface intentionally precedes Python). Throws `SpendGuardError("query_budget not yet wired in sidecar; tracked at <GH issue>")`.
3. Multi-event commit: extend `commitEstimated()` (or add `commitActual()`) to also write a single `LLM_CALL_OUTCOME` event when caller has actuals — matches Python `_LoopBoundCallback.async_log_success_event` shape.

Plus the gRPC Status → typed-error mapping for the `FailedPrecondition` cluster: `IDEMPOTENCY_CONFLICT`, `BUDGET_EXCEEDED`, `BUNDLE_HOT_RELOADED`, `MUTATION_APPLY_FAILED`. These map onto existing `errors.ts` classes (`MutationApplyFailed`, `ApprovalBundleHotReloadedError`, etc.).

Concretely:
- `sdk/typescript/src/client.ts`:
  - **`release(req: ReleaseRequest)`**: real `adapterClient.releaseReservation({...})` call. Maps `ReleaseRequest` → proto wire shape (sessionId reused from handshake state per design §3). Returns `ReleaseOutcome` with `decisionId` + `releaseId` + `releasedAtAtomic`. On gRPC Status NOT_FOUND → throw `SpendGuardError("reservation not found")`. On FAILED_PRECONDITION + `details.code === "IDEMPOTENCY_CONFLICT"` → throw `MutationApplyFailed` with original details. Disabled-mode short-circuit per implementation.md §4.
  - **`queryBudget(req)`**: throws `SpendGuardError("query_budget not yet wired in sidecar; tracked at <issue>")` — this is the §9.4 placeholder; tests assert the throw shape. Disabled-mode returns `makeDisabledQueryBudgetResult(req)` (mirroring SLICE 4 helpers).
  - **Multi-event commit**: extend `commitEstimated()` signature to accept optional outcome fields (`outcomeKind?: "SUCCESS" | "FAILURE"`, `actualInputTokens?: string`, `actualOutputTokens?: string`, `actualErrorMessage?: string`). When `outcomeKind` is present, emit BOTH `LLM_CALL_POST` AND `LLM_CALL_OUTCOME` events on the same bidi stream. When absent, behavior unchanged from SLICE 4 (single LLM_CALL_POST). All forward verbatim to wire.
  - **gRPC Status → typed-error mapper**: central `mapGrpcStatusToError(status: ServerStatus, ctx: RpcContext): SpendGuardError` helper. Handles UNAVAILABLE → SidecarUnavailable; DEADLINE_EXCEEDED → SidecarUnavailable with code; FAILED_PRECONDITION with details code dispatch (IDEMPOTENCY_CONFLICT, BUNDLE_HOT_RELOADED, BUDGET_EXCEEDED); ABORTED → SpendGuardError. Used by all 4 wired RPCs (handshake, reserve, commitEstimated, release).
  - Remove `SLICE_5_NOT_WIRED` constant — no longer referenced from release/queryBudget. Other 4 stubs (`confirmPublishOutcome`, `resumeAfterApproval`, `safeConfirmApplyFailed`, `emitLlmCallPost`) remain as SLICE 7+ stubs but with explicit `SLICE_7_NOT_WIRED` per their respective slice plan rows.
  - **File header cleanup (M-1 from SLICE 4 R1)**: update preamble from "(SLICE 3)" to "(SLICE 3 + SLICE 4 + SLICE 5)" + move SLICE 6/7/8 deferrals to "future slices" block.

- `sdk/typescript/src/errors.ts`:
  - Add explicit re-export of `MutationApplyFailed`, `ApprovalBundleHotReloadedError` from index barrel if not already.
  - Add JSDoc on each error class explaining when the typed-error mapper raises it.

- `sdk/typescript/tests/_support/mockSidecar.ts`:
  - Add `releaseReservation` handler returning shaped ReleaseOutcome.
  - Add `releaseReservationFailing` handler factory returning configurable gRPC Status + details (for typed-error mapper tests).
  - Extend `emitTraceEvents` mock to also accept multi-event (LLM_CALL_POST + LLM_CALL_OUTCOME) streams; captures both events for assertion.

- `sdk/typescript/tests/release-query.test.ts` — NEW:
  - release() success → ReleaseOutcome with decisionId + releaseId
  - release() with NOT_FOUND → SpendGuardError("reservation not found")
  - release() with FAILED_PRECONDITION+IDEMPOTENCY_CONFLICT → MutationApplyFailed
  - release() in disabled mode → makeDisabledReleaseOutcome short-circuit
  - queryBudget() always throws "query_budget not yet wired"
  - queryBudget() in disabled mode → makeDisabledQueryBudgetResult
  - commitEstimated() with outcome → 2 events emitted (LLM_CALL_POST + LLM_CALL_OUTCOME)
  - commitEstimated() without outcome → 1 event emitted (SLICE 4 regression)
  - mapGrpcStatusToError: UNAVAILABLE → SidecarUnavailable; FAILED_PRECONDITION+BUNDLE_HOT_RELOADED → ApprovalBundleHotReloadedError; etc.
  - mapGrpcStatusToError: FAILED_PRECONDITION+unknown details code → MutationApplyFailed (default)

- `sdk/typescript/tests/locked-surface.test.ts`:
  - Add release / queryBudget signature assertions per §4.2 LOCKED surface table.
  - Assert `commitEstimated` accepts optional outcome params (type-level test via `AssertMutuallyAssignable`).

## Files touched

| File | Why |
|------|-----|
| `sdk/typescript/src/client.ts` | release() / queryBudget() / commitEstimated() multi-event / gRPC mapper |
| `sdk/typescript/src/errors.ts` | JSDoc + barrel re-exports |
| `sdk/typescript/tests/_support/mockSidecar.ts` | release handlers + multi-event capture |
| `sdk/typescript/tests/release-query.test.ts` | NEW — release/query/multi-event/error-map tests |
| `sdk/typescript/tests/locked-surface.test.ts` | Surface assertions for release/query/outcome |
| `sdk/typescript/src/index.ts` | Re-export new types if needed |

## Test/verification plan

1. `pnpm run typecheck` clean (both src + tests configs).
2. `pnpm run test` — 133 + ~18 new = ~151 passing.
3. `pnpm run build` clean; `dist/index.js` ≤ 120 KB §4.2 budget.
4. `pnpm run lint` clean (biome).
5. Key new tests:
   - release() success + decisionId/releaseId forwarding
   - release() IDEMPOTENCY_CONFLICT → MutationApplyFailed
   - queryBudget() placeholder throw shape (per design §9.4)
   - commitEstimated() multi-event mode emits 2 events
   - mapGrpcStatusToError exhaustive coverage of FAILED_PRECONDITION details

## Anti-scope

- No `confirmPublishOutcome` / `resumeAfterApproval` body — SLICE 6 or 7.
- No `safeConfirmApplyFailed` / `emitLlmCallPost` body — SLICE 6 or 7.
- No `ids.ts` / `promptHash.ts` / `pricing.ts` — SLICE 6.
- No `withRunPlan` — SLICE 7.
- No OTel / retry / idempotency cache — SLICE 8.
- No actual sidecar-side queryBudget RPC — that ships in `services/sidecar` independently of this SDK slice (per design §9.4).

## Backlinks

- Spec set: [`design.md`](../../specs/coverage/D05_ts_sdk_substrate/design.md) §4.4 release/query/commit signatures, §4.5 error hierarchy, §8 slice 5 row, §9.4 queryBudget deferral
- SLICE 4 R1 residuals: M-1 (header), M-2 (empty-string policy), M-3 (promptText), M-4 (bundleSignature) — fold M-1 here; M-2/M-3/M-4 stay deferred to SLICE 6+
- SLICE 4: [`COV_S05_04_d05_handshake_reserve_commit.md`](COV_S05_04_d05_handshake_reserve_commit.md)
