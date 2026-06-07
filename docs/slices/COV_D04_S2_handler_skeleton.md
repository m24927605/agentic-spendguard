# COV_D04_S2 — D04 LangChain TS: SpendGuardCallbackHandler skeleton

> **Deliverable**: D04 LangChain TS adapter
> **Slice**: 2 of 6 (S)

## Scope

Add `SpendGuardCallbackHandler` class extending `BaseCallbackHandler` from `@langchain/core/callbacks/base`. PRE/POST hooks stubbed (throw NotImplementedError until SLICE 3). Options + per-call inflight Map.

Concretely:
- `sdk/typescript-langchain/src/handler.ts` — NEW class with:
  - `extends BaseCallbackHandler` (LangChain's standard base)
  - `name = "spendguard_callback_handler"`
  - constructor accepts `{ client: SpendGuardClient, ... }` options
  - `inflight: Map<string, { decisionId, reservationId }>` per-call state
  - `handleChatModelStart(llm, messages, runId, parentRunId, extraParams, tags, metadata, name?)` — PRE stub
  - `handleLLMEnd(output, runId, parentRunId, tags)` — POST success stub
  - `handleLLMError(err, runId, parentRunId, tags)` — POST failure stub
- `sdk/typescript-langchain/src/options.ts` — NEW SpendGuardCallbackHandlerOptions type
- `sdk/typescript-langchain/src/errors.ts` — re-export from @spendguard/sdk
- `sdk/typescript-langchain/src/index.ts` — barrel: SpendGuardCallbackHandler + options
- ≥8 tests covering: name property, constructor + options, inflight Map, hook signatures match LangChain's BaseCallbackHandler

## Anti-scope

- No reserve/commit wiring — SLICE 3
- No mock sidecar tests — SLICE 4
- No demo — SLICE 5
- No docs/publish — SLICE 6
