# D04 — Review Standards

Use this checklist with `superpowers:code-reviewer` on every D04 slice. R1 runs the full checklist; R2-R5 focus on findings still open from the previous round. Findings are categorised P0 / P1 / P2 / Polish; P0 + P1 are blockers.

## 1. Public-surface lock (P0 — blocker)

The public surface is the contract consumers depend on. Drift after `design.md` is merged requires a re-spec.

| Check | Pass condition |
|---|---|
| 1.1 | `src/index.ts` exports `SpendGuardCallbackHandler` and the `SpendGuardCallbackHandlerOptions` type — and ONLY these |
| 1.2 | `SpendGuardCallbackHandler extends BaseCallbackHandler` from `@langchain/core/callbacks/base` |
| 1.3 | `awaitHandlers = true` and `raiseError = true` are set on the instance — required for throw propagation per `@langchain/core@0.3` |
| 1.4 | `static lc_name()` returns `"SpendGuardCallbackHandler"` for LangChain serialization |
| 1.5 | `SpendGuardCallbackHandlerOptions` mirrors `design.md` §4 field-for-field |
| 1.6 | Naming: camelCase on the public surface; no snake_case anywhere outside generated proto types |
| 1.7 | No `default export` in `src/index.ts` |
| 1.8 | Adapter does NOT re-export `@spendguard/sdk` symbols — consumer imports directly |

If any of 1.1–1.8 fail → P0.

## 2. LangChain protocol correctness (P0 — blocker)

| Check | Pass condition |
|---|---|
| 2.1 | `handleChatModelStart` signature matches `@langchain/core@0.3` exactly: `(serialized, messages, runId, parentRunId?, extraParams?, tags?, metadata?, runName?)` |
| 2.2 | `handleLLMStart` signature matches the prompts-array variant |
| 2.3 | `handleLLMEnd(output: LLMResult, runId: string, parentRunId?, tags?)` matches |
| 2.4 | `handleLLMError(err, runId, parentRunId?, tags?)` matches |
| 2.5 | Inline-mode dispatch is verified: throwing inside `handleChatModelStart` halts the consumer's `await model.invoke()` (test E-01..E-03 prove this) |
| 2.6 | `runId` from LangChain (RunManager UUID) is used as `llmCallId` — **never re-generated** |
| 2.7 | `parentRunId` is forwarded to `reserve` |
| 2.8 | Streaming `handleLLMNewToken` is NOT wired (PRE+POST only in v0.1.0) |
| 2.9 | Streaming `handleLLMEnd` fires once per stream completion; `commitEstimated` fires exactly once |
| 2.10 | The handler is reusable across multiple invocations without per-call construction |

## 3. Reserve / commit semantics (P0 — blocker)

| Check | Pass condition |
|---|---|
| 3.1 | `client.reserve` is called with `trigger=LLM_CALL_PRE` |
| 3.2 | `llmCallId = runId` (LangChain's run ID) — exact equality |
| 3.3 | `decisionId = deriveUuidFromSignature(signature, { scope: "decision_id" })` |
| 3.4 | `idempotencyKey = deriveIdempotencyKey({tenantId, sessionId, runId, stepId, llmCallId, trigger: "LLM_CALL_PRE"})` |
| 3.5 | `stepId` matches Python's shape `${runId}:lc:${signature.slice(0,16)}` — preserves parity for cross-language idempotency-key generation |
| 3.6 | `projectedClaims` come from `claimEstimator(input)` — invoked exactly once per reserve |
| 3.7 | `claimEstimate` (optional) is forwarded verbatim |
| 3.8 | On success, the in-flight record is stored keyed by `runId` |
| 3.9 | `handleLLMEnd` calls `commitEstimated` with `outcome="SUCCESS"` and `estimatedAmountAtomic` from the extracted token total |
| 3.10 | `handleLLMError` calls `commitEstimated` with `outcome="PROVIDER_ERROR"` and `estimatedAmountAtomic="0"` |
| 3.11 | `handleLLMEnd` / `handleLLMError` for an unknown `runId` is a no-op (does NOT throw) |
| 3.12 | `route` defaults to `"llm.call"`; consumer-provided value propagates |

## 4. Error propagation (P0)

| Check | Pass condition |
|---|---|
| 4.1 | `DecisionStopped` thrown from `reserve` propagates out of `model.invoke()` without being swallowed |
| 4.2 | `DecisionDenied`, `DecisionSkipped`, `SidecarUnavailable` all propagate identically |
| 4.3 | `ApprovalRequired` without `onApprovalRequired` propagates |
| 4.4 | `ApprovalRequired` with `onApprovalRequired` that returns a `DecisionOutcome` resumes — the inflight record stores the resumed outcome |
| 4.5 | `ApprovalRequired` with `onApprovalRequired` returning null/undef propagates the original error |
| 4.6 | A throw in `claimEstimator` propagates — not silently swallowed |
| 4.7 | Test EE-02 (denied → 0 fetch calls) is real: the `fetch` spy records 0 calls when `reserve` throws |

## 5. Inflight correlation (P1)

| Check | Pass condition |
|---|---|
| 5.1 | `InflightMap` keys by `runId` only — no global shared state |
| 5.2 | Capacity bounded at 10 k entries; FIFO eviction on overflow |
| 5.3 | `take(runId)` returns + deletes in one op |
| 5.4 | Concurrent invocations with distinct `runId`s do not cross-talk |
| 5.5 | A handler instance reused across multiple model invocations cleans up entries correctly |

## 6. Token-usage extraction (P1)

| Check | Pass condition |
|---|---|
| 6.1 | Reads `usage_metadata.total_tokens` first (LangChain 0.3 path) |
| 6.2 | Falls back to `response_metadata.token_usage.total_tokens` (older path) |
| 6.3 | Returns `0` when neither present — never throws |
| 6.4 | `provider_event_id` reads `response_metadata.id` first, then `response_metadata.response_id`, else `""` |
| 6.5 | Robust to `usage_metadata` being a non-object (handles LangChain minor drift) |
| 6.6 | Extraction logic mirrors Python `langchain.py:340-368` semantically (parity verified) |

## 7. ESM-only + tree-shakeability (P1)

| Check | Pass condition |
|---|---|
| 7.1 | `"type": "module"` in `package.json` |
| 7.2 | No CJS build artefact in `dist/` |
| 7.3 | `"sideEffects": false` |
| 7.4 | `treeShaking.test.ts` confirms `dist/index.js` does NOT statically import `@grpc/grpc-js` (substrate owns that) |
| 7.5 | tsup config produces ESM-only output |
| 7.6 | Type-only imports from `@spendguard/sdk` use `import type` syntax — no runtime cost for unused types |

## 8. Bundle-size budget (P1)

| Check | Pass condition |
|---|---|
| 8.1 | `pnpm run size` script exists and is wired into `prepack` |
| 8.2 | `dist/index.js` minified ≤ 40 KB |
| 8.3 | `dist/index.js` gzipped ≤ 12 KB |
| 8.4 | Budget breach is a build failure, not a warning |

## 9. Cross-language idempotency parity (P0)

| Check | Pass condition |
|---|---|
| 9.1 | `deriveIdempotencyKey` output for the same `(tenantId, sessionId, runId, stepId, llmCallId)` tuple is byte-identical to Python `derive_idempotency_key(...)` |
| 9.2 | The shared fixture `sdk/fixtures/cross-language/langchain_v1.json` is committed |
| 9.3 | Both the TS and Python parity tests consume the same fixture |
| 9.4 | `stepId` shape matches Python exactly: `${runId}:lc:${signature.slice(0, 16)}` |
| 9.5 | `defaultCallSignature` invocation parity holds when both adapters see the same canonicalised inputs |

Drift here breaks audit-chain dedup across Python + TS LangChain users. P0 — blocker.

## 10. Streaming behaviour (P1)

| Check | Pass condition |
|---|---|
| 10.1 | `model.stream()` triggers `handleChatModelStart` once at stream open |
| 10.2 | `reserve` fires before the first SSE chunk leaves the provider |
| 10.3 | `handleLLMNewToken` events do NOT call `reserve` again |
| 10.4 | `handleLLMEnd` fires once at stream completion → exactly one `commitEstimated` |
| 10.5 | Aborted stream → `handleLLMError` → one PROVIDER_ERROR commit |

## 11. Demo correctness (P1)

| Check | Pass condition |
|---|---|
| 11.1 | `examples/langchain-ts/src/agent_real_langchain_ts.ts` connects to the demo sidecar UDS and runs `ChatOpenAI.invoke()` |
| 11.2 | `make demo-up DEMO_MODE=agent_real_langchain_ts` exits 0; sidecar logs one RequestDecision ack |
| 11.3 | The demo container has a Node 20 base image stage and `@spendguard/langchain` + `@langchain/openai` installed at build time |
| 11.4 | `deploy/demo/demo/run_demo.py` dispatches `agent_real_langchain_ts` to a `subprocess.run(["node", ...])` invocation |
| 11.5 | Denied-budget run produces 0 OpenAI HTTP requests (proven via the egress-proxy or fetch-log assertion) |
| 11.6 | Audit row ordering: `LLM_CALL_PRE` `created_at` < first OpenAI request timestamp |
| 11.7 | OPENAI_API_KEY missing → demo aborts with a clear error (matches Python langchain demo's behavior) |

## 12. Documentation completeness (P2)

| Check | Pass condition |
|---|---|
| 12.1 | `sdk/typescript/integrations/langchain/README.md` 30-line quickstart works as-is |
| 12.2 | Every public method has JSDoc with `@throws` block enumerating typed exceptions |
| 12.3 | `CHANGELOG.md` 0.1.0 entry calls out: "TS counterpart of `spendguard-sdk[langchain]` (Python) v0.5.1; callback-handler shape" |
| 12.4 | `LICENSE_NOTICES.md` lists `@langchain/core` (MIT) and `@spendguard/sdk` (Apache-2.0) |
| 12.5 | `docs/site/docs/integrations/langchain.md` updated with a TS section beneath the existing Python section |
| 12.6 | `README.md` (repo root) `## Adapter integrations` table has the `@spendguard/langchain` row |

## 13. Security (P1)

| Check | Pass condition |
|---|---|
| 13.1 | No `eval`, `new Function`, or `Function.prototype.constructor` anywhere |
| 13.2 | Handler never logs prompts at INFO level — only at TRACE (and only when explicitly configured) |
| 13.3 | `parentRunId`, `tags`, `metadata` from LangChain are NOT deep-cloned into logs (could leak PII) |
| 13.4 | `runId` is treated as opaque — no parsing, no eval |
| 13.5 | `claimEstimator` is called with a frozen-by-convention input object — adapter does not mutate user-provided structures |
| 13.6 | `npm audit --omit=dev` reports 0 high/critical advisories at publish time |

## 14. Publish pipeline (P1)

| Check | Pass condition |
|---|---|
| 14.1 | `.github/workflows/sdk-ts-langchain-publish.yml` exists |
| 14.2 | Triggered on `release` event + `workflow_dispatch` |
| 14.3 | `if: startsWith(github.ref, 'refs/tags/langchain-ts-v')` gates the publish job |
| 14.4 | `permissions: id-token: write` set on the publish job (OIDC) |
| 14.5 | `npm publish --provenance --access public` |
| 14.6 | Workflow runs lint, typecheck, test, build, size before publish |
| 14.7 | Lockfile-frozen install (`pnpm install --frozen-lockfile`) |

## 15. Slice-specific anti-scope

| Slice | Anti-scope check |
|---|---|
| `COV_D04_01_pkg_init` | No source files beyond placeholder `src/index.ts`; no tests beyond sanity import |
| `COV_D04_02_handler_skeleton` | Handler stubs reserve/commit — no real client call yet; events recorded into inflight map only |
| `COV_D04_03_reserve_commit_wiring` | No streaming-specific code; no demo script; no docs page |
| `COV_D04_04_tests_mock_sidecar` | No source changes beyond test helpers; only tests added |
| `COV_D04_05_demo_agent_real_langchain_ts` | Demo script + Makefile + run_demo.py dispatch only; no handler source changes |
| `COV_D04_06_docs_publish` | No source changes; only README, CHANGELOG, LICENSE_NOTICES, docs site page, repo-root adapter table, publish workflow |

## 16. Findings categorisation

| Category | Definition | R1 action |
|---|---|---|
| **P0** | Public-surface drift, throw propagation broken, cross-language idempotency drift, security finding | Block. Fix before re-run. |
| **P1** | Spec gate failure, missing test, missing documentation, wrong error class | Block. Fix before re-run. |
| **P2** | Stylistic, minor JSDoc gap, non-critical perf, polish | Track as residual; may merge with note. |
| **Polish** | Naming preferences, comment wording | Track as residual; do not block. |

## 17. R1-R5 escalation rules

- Same finding in two consecutive rounds without progress → Staff+ panel arbitration per build-plan §1.3.
- P0 finding open at R5 → automatic Staff+ panel arbitration.
- Deferred P2/Polish residuals filed as `gh issue` referenced from the slice doc.

## 18. Residual triage template

```
Title: [D04 residual] <one-line summary>

Body:
- Slice: COV_D04_<NN>_<short>
- Round: R<n>
- Category: P<0|1|2>|Polish
- Spec ref: design.md §<n>, tests.md §<n>, acceptance.md §<n>
- Repro: <minimal command sequence>
- Why deferred: <one line>
- Suggested follow-up slice: <name or "TBD post-D04">
```

## 19. Sign-off

The reviewer signs off only when:
- Every P0 + P1 in §1–§14 is green.
- Slice-specific anti-scope in §15 is honored.
- All R<=5 findings are resolved or filed as residuals.
- Acceptance gates in `acceptance.md` §12 are green.

If any of those fail → slice does not pass R review → loop continues.
