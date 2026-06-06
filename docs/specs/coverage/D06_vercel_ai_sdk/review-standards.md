# D06 — Review Standards

Use this checklist with `superpowers:code-reviewer` on every D06 slice. R1 runs the full checklist; R2-R5 focus only on findings still open from the previous round. Findings are categorised P0 / P1 / P2 / Polish; P0 + P1 are blockers.

## 1. Public-surface lock (P0 — blocker)

The middleware is the contract Vercel AI SDK and Mastra users build against. Changes to the public surface after `design.md` is merged require a re-spec and a v0.minor bump.

| Check | Pass condition |
|---|---|
| 1.1 | `createSpendGuardMiddleware(opts: SpendGuardMiddlewareOptions): LanguageModelV2Middleware` exported from `src/index.ts` with this exact signature. |
| 1.2 | `SpendGuardMiddlewareOptions` has the fields listed in `design.md` §4 — `client`, `budgetId`, `windowInstanceId`, `unit`, `pricing` (required), and `claimEstimator`, `callSignature`, `runIdProvider`, `route`, `providerEventIdExtractor` (optional). No field renamed, no field added without a v0.minor bump. |
| 1.3 | `wrapWithSpendGuard(model, opts)` shorthand exported. |
| 1.4 | `@spendguard/vercel-ai/mastra` subpath exports `createSpendGuardLanguageMiddleware` as a **function-reference alias** (not a copy) of `createSpendGuardMiddleware`. The test at 1.6 enforces this. |
| 1.5 | No `default export` in any file under `src/`. |
| 1.6 | `tests/mastra/agent.test.ts` case 5.6 asserts `createSpendGuardMiddleware === createSpendGuardLanguageMiddleware` (strict equality). |
| 1.7 | `package.json#exports` map matches the subpaths in `implementation.md` §2 exactly. |

If any of 1.1–1.7 fail → P0. The contract D06 advertises to D04/D08/D29 consumers is broken.

## 2. AI SDK v5 LanguageModelV2 conformance (P0 — blocker)

The middleware MUST implement `LanguageModelV2Middleware` from `@ai-sdk/provider` correctly. Any conformance violation breaks the entire integration.

| Check | Pass condition |
|---|---|
| 2.1 | `transformParams({ type, params })` returns the (possibly modified) `params` — never `undefined`, never throws on the happy path. |
| 2.2 | `wrapGenerate({ doGenerate, params })` awaits `doGenerate()` and returns its result type-equivalently — no shape mutation. |
| 2.3 | `wrapStream({ doStream, params })` returns `{ stream, request, response, warnings }` — all four keys preserved from inner. Only `stream` is replaced. |
| 2.4 | Replaced `stream` is a `ReadableStream<LanguageModelV2StreamPart>` (not an iterator, not a generator wrapper). |
| 2.5 | Type test `tests/types.test-d.ts` 10.1 (`expectTypeOf(createSpendGuardMiddleware(...)).toEqualTypeOf<LanguageModelV2Middleware>()`) passes. |
| 2.6 | `wrapLanguageModel({ model: openai("..."), middleware: createSpendGuardMiddleware(...) })` typechecks against `ai@5` — no `as any` workarounds. |

## 3. Streaming-completion correctness (P0 — blocker)

Streaming COMMIT MUST fire when stream completes, never before. Mid-stream cancellation MUST roll back via `release()`. This is the user-stated acceptance criterion; failure = ship-block.

| Check | Pass condition |
|---|---|
| 3.1 | Commit RPC is issued only after the `finish` part has been processed by the TransformStream `flush()` handler. Verified by mock-sidecar timing in tests 3.1 / 4.5. |
| 3.2 | Mid-stream cancellation (`stream.cancel()`) triggers `release()` exactly once. Tests 3.3 / 4.6. |
| 3.3 | Stream-side errors propagate to the consumer downstream (consumer must see the error). Test 3.2. |
| 3.4 | Commit-side failure (e.g. sidecar UNAVAILABLE during the post-finish commit) does NOT corrupt the stream — consumer sees the `finish` part forwarded successfully. Test 3.4. |
| 3.5 | Terminal-state race (finish AND cancel land simultaneously) is single-shot — exactly one of `onFinish` / `onError` fires. Test 3.5. |
| 3.6 | Empty-stream case (provider returns no parts) emits `commit` with `totalTokens=0` rather than skipping. Test 3.6. |

## 4. Idempotency under retries (P1)

The Vercel AI SDK's internal retry loop (`maxRetries` default 2 in v5) re-enters `transformParams` with identical params. The middleware MUST produce identical `idempotencyKey` so the sidecar collapses the retry.

| Check | Pass condition |
|---|---|
| 4.1 | Identity derivation is pure: same params + same runId → same `idempotencyKey`. Test 6.1. |
| 4.2 | Sidecar dedup collapses: across a 2-retry scenario, exactly one COMMIT lands on the sidecar (not 2). Test 6.2. |
| 4.3 | After full retry exhaustion: exactly one RELEASE lands. Test 6.3. |
| 4.4 | `defaultParamsSignature` uses `defaultCallSignature` from D05 — never a local re-implementation. Diff inspection. |

## 5. Cross-language determinism (P0 — inherited from D05)

| Check | Pass condition |
|---|---|
| 5.1 | `idempotencyKey` derived from the same `(messages, settings)` is byte-equal to the Python `pydantic_ai.py` adapter's derivation. Test 2.7 reads `sdk/fixtures/cross-language/v1.json`. |
| 5.2 | The middleware does NOT have its own `computePromptHash` or `deriveIdempotencyKey` — it imports them from `@spendguard/sdk`. Diff inspection: zero `crypto.createHmac` or `crypto.createHash` calls anywhere under `src/`. |
| 5.3 | The middleware does NOT canonicalise messages itself for hashing — it delegates to D05's `defaultCallSignature`. Diff inspection. |

Drift here breaks audit-chain dedup across Python + TS estate. P0.

## 6. Run-context propagation (P1)

| Check | Pass condition |
|---|---|
| 6.1 | When `runIdProvider` is passed, it wins over `currentRunPlan()`. Test 1.8. |
| 6.2 | When neither is set, the first `transformParams` call throws `SpendGuardConfigError` (fail-fast). Test 1.7. |
| 6.3 | RunPlan binding via `await withRunPlan({runId}, () => generateText(...))` propagates `runId` to the sidecar reserve call. Test 7.1. |
| 6.4 | Nested calls inside one RunPlan share `runId` but differ in `stepId`. Test 7.2. |
| 6.5 | Concurrent `withRunPlan` blocks via `Promise.all` do not leak runIds. Test 7.3. |
| 6.6 | `traceparent`, `tracestate`, `parentRunId`, `budgetGrantJti` are forwarded to the sidecar reserve call when set on the RunPlan. |

## 7. Error handling + denial propagation (P1)

| Check | Pass condition |
|---|---|
| 7.1 | A `DecisionDenied` thrown by `client.reserve(...)` propagates from `transformParams` unwrapped — the Vercel AI SDK caller catches the exact D05 error type. Test 4.3. |
| 7.2 | `DecisionStopped` (a subclass of `DecisionDenied`) propagates identically; the inner `doGenerate` is NEVER called when reserve denies. Test 4.2. |
| 7.3 | `ApprovalRequired` propagates; the user can call `.resume(client)` exactly as D05 documents. |
| 7.4 | Provider error during `doGenerate` calls `release(...)` exactly once with `reasonCode="PROVIDER_ERROR"`. Test 4.4. |
| 7.5 | Stream-side error calls `release(...)` exactly once with `reasonCode="PROVIDER_ERROR"`. Test 4.6. |
| 7.6 | When `outcome.reservationIds` is empty (sidecar approved without reservation, e.g. SKIP), commit short-circuits to `confirmPublishOutcome` with `APPLIED_NOOP` — no LLM_CALL_POST emitted. Mirror of `pydantic_ai.py:614-622`. |
| 7.7 | `MutationApplyFailed` is raised (not silently swallowed) when reserve returns DEGRADE with a non-empty patch — for now (v0.1) DEGRADE patch application is out of scope. |

## 8. WeakMap stash discipline (P1)

| Check | Pass condition |
|---|---|
| 8.1 | The stash is a module-level `WeakMap<LanguageModelV2CallOptions, StashEntry>`, not a global `Map` and not `Object.assign` on params. |
| 8.2 | Stash keys are the params object reference — never a string-serialised key. |
| 8.3 | `wrapGenerate` / `wrapStream` throw a clear error when no stash entry exists (developer forgot `wrapLanguageModel` composition). Test 1.5 / 1.6. |
| 8.4 | No memory leak: GC of the params object should make the stash entry collectable (asserted by holding only a `WeakRef` in test and forcing GC). |

## 9. Bundle size + tree-shaking (P1)

| Check | Pass condition |
|---|---|
| 9.1 | `dist/index.js` minified ≤ 30 KB (excluding peer-dep externals). |
| 9.2 | `dist/index.js` gzipped ≤ 12 KB. |
| 9.3 | `pnpm run size` script exists and runs in `prepack`. |
| 9.4 | A budget breach is a build failure, not a warning. |
| 9.5 | `@spendguard/sdk` and `ai` are correctly externalised — `dist/index.js` does not inline their code. Tests 9.1 / 9.3. |
| 9.6 | `"sideEffects": false` in `package.json`. |

## 10. Provider matrix coverage (P1)

| Check | Pass condition |
|---|---|
| 10.1 | Tests against `@ai-sdk/openai@^1` and `@ai-sdk/anthropic@^1` pass — both providers, all 6 cases each. |
| 10.2 | Tests use recorded JSON fixtures, not live API keys. CI logs grep for `_API_KEY` shows zero references in source. |
| 10.3 | Mock sidecar in `tests/_support/mockSidecar.ts` is `@grpc/grpc-js` UDS-based — matches the production transport. |

## 11. Mastra coverage gate (P1)

| Check | Pass condition |
|---|---|
| 11.1 | `tests/mastra/agent.test.ts` constructs an actual `@mastra/core` Agent with the wrapped model and runs both `agent.generate(...)` and `agent.stream(...)`. |
| 11.2 | Demo `agent_real_mastra` uses the documented Mastra entry path (`@spendguard/vercel-ai/mastra`). |
| 11.3 | `docs/site/docs/integrations/vercel-ai-and-mastra.md` has both quickstarts side-by-side; reviewers should be able to copy-paste either and have it work. |
| 11.4 | README and CHANGELOG mention Mastra explicitly as a covered ecosystem. |

## 12. ESM-only enforcement (P1)

| Check | Pass condition |
|---|---|
| 12.1 | `"type": "module"` in `package.json`. |
| 12.2 | No CJS output in `dist/`. |
| 12.3 | tsup config: `format: ["esm"]` only. |
| 12.4 | No `require(...)` calls anywhere in `src/`. |

## 13. Demo regression gate (P0)

| Check | Pass condition |
|---|---|
| 13.1 | `make demo MODE=agent_real_vercel_ai_ts` exits 0 and emits a full reserve → generate → commit cycle. |
| 13.2 | `make demo MODE=agent_real_vercel_ai_ts_stream` exits 0 and the commit timestamp follows the final stream chunk timestamp. |
| 13.3 | `make demo MODE=agent_real_mastra` exits 0 with the same reserve → generate → commit cycle visible. |
| 13.4 | Demo scripts under `deploy/demo/scripts/agent_real_vercel_ai_ts/` are committed and deterministic — no random seeds, fixture-driven. |

A demo gate failure is ALWAYS a P0 even if all unit tests pass — per `feedback_demo_quality_gate` ("demo as quality gate"). Codex ✅ is not enough; the demo must really run.

## 14. Documentation completeness (P2)

| Check | Pass condition |
|---|---|
| 14.1 | README has both Vercel AI SDK and Mastra quickstarts. |
| 14.2 | JSDoc on `createSpendGuardMiddleware` describes both ecosystems. |
| 14.3 | Streaming semantics (commit-after-finish, cancel-rolls-back) is documented in README + integrations page. |
| 14.4 | RunPlan requirement is documented prominently — no silent runId failure mode in user-facing copy. |
| 14.5 | Troubleshooting section: how to debug "wrapGenerate called without transformParams" and "no SpendGuard RunContext bound" errors. |

## 15. Aspects out of scope for v0.1 — verify NOT shipped (P2)

| Check | Pass condition |
|---|---|
| 15.1 | No tool-call gating (`TOOL_CALL_PRE`) — verify no `TOOL_CALL_PRE` strings under `src/`. |
| 15.2 | No DEGRADE patch application — verify `MutationApplyFailed` thrown on non-empty patch. |
| 15.3 | No v4 back-compat code — verify no `experimental_wrapLanguageModel` references. |
| 15.4 | No live provider keys used in tests — grep confirms. |

These are explicit anti-scope items from `design.md` §3 — shipping any of them in v0.1 is a polish-level finding (P2) requiring justification before merge.

## 16. Findings escalation per build plan §1.1

- R1-R5 loop. Reviewer flags findings; same Staff+ implementer fixes via SendMessage.
- R5 findings > 0 → Staff+ panel arbitration (Software Architect + Backend Architect + AI Engineer + Security Engineer + Senior Developer).
- Summarizer (Software Architect by default) reconciles → final ruling: `merge-with-residuals` | `block` | `rework`.
- Residuals tracked as GH issues per `feedback_codex_iteration_pattern`.
