# COV_D04_S3 — D04 LangChain TS: reserve/commit wiring

> **Deliverable**: D04 LangChain TS
> **Slice**: 3 of 6 (M)

## Scope

Wire real bodies for SpendGuardCallbackHandler's 3 hook stubs:
- `handleChatModelStart`: derive idempotencyKey from runId+parentRunId, build ReserveRequest, call client.reserve(). On DENY → throw DecisionDenied (LangChain propagates as run-level error). Stash decisionId+reservationId in inflight Map keyed by runId.
- `handleLLMEnd`: read inflight entry by runId, build CommitEstimatedRequest with actual token usage from LLMResult.llmOutput.tokenUsage, call client.commitEstimated() with outcomeKind=SUCCESS. Clear inflight entry.
- `handleLLMError`: read inflight entry by runId, call client.commitEstimated() with outcomeKind=FAILURE + actualErrorMessage. Clear inflight entry.

PROVIDER_ERROR path: when LangChain hands us an LLMResult with error metadata or handleLLMError fires, commit with FAILURE outcome.

Concretely:
- `sdk/typescript-langchain/src/handler.ts` — wire 3 hook bodies
- `sdk/typescript-langchain/src/ids.ts` — NEW small helper deriveIdempotencyKey wrapping @spendguard/sdk's deriveIdempotencyKey for LangChain's runId/parentRunId shape
- ≥15 tests covering: reserve success → inflight stashed, reserve DENY → throw DecisionDenied, end → commit SUCCESS + inflight cleared, error → commit FAILURE + inflight cleared, missing inflight on END → warn (already committed elsewhere), token usage extraction from LLMResult, multiple concurrent runs (different runIds) don't collide

## Anti-scope

- No mock sidecar tests — SLICE 4
- No examples/langchain-ts demo — SLICE 5
- No docs page — SLICE 6
