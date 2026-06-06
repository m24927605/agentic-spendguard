# D29 — Review Standards

Use this checklist with `superpowers:code-reviewer` on every D29 slice. R1 runs the full checklist; R2-R5 focus on findings still open from the previous round. Findings are categorised P0 / P1 / P2 / Polish; P0 + P1 are blockers.

## 1. Public-surface lock (P0 — blocker)

The public surface is the contract consumers depend on. Drift after `design.md` is merged requires a re-spec.

| Check | Pass condition |
|---|---|
| 1.1 | `src/index.ts` exports `wrapWithSpendGuard` (function) and `WrapOptions`, `ClaimEstimatorInput`, `ClaimEstimator` (types) — and ONLY these |
| 1.2 | `wrapWithSpendGuard(stepAi, client, options)` is the only runtime export |
| 1.3 | Returned object's `infer` / `wrap` signatures match `@inngest/agent-kit@^0.1`'s `step.ai` exactly (type-preservation gate) |
| 1.4 | `WrapOptions` mirrors `design.md` §4 field-for-field |
| 1.5 | Naming: camelCase on the public surface; no snake_case outside generated proto types |
| 1.6 | No `default export` in `src/index.ts` |
| 1.7 | Adapter does NOT re-export `@spendguard/sdk` symbols — consumer imports directly |

If any of 1.1–1.7 fail → P0.

## 2. Inngest protocol correctness (P0 — blocker)

| Check | Pass condition |
|---|---|
| 2.1 | The wrap intercepts BOTH `step.ai.infer` and `step.ai.wrap` (the two AgentKit primitives) |
| 2.2 | The original `stepAi.infer` body is invoked exactly once per `sgStep.infer` call (no double-invocation) |
| 2.3 | The runtime context (`{runId, eventId, step}`) is forwarded to the original `stepAi.infer` untouched |
| 2.4 | When `runtimeCtx` is undefined (test harness path), adapter degrades gracefully — uses `name` as `stepId`, empty string `runId` — and documents this in JSDoc |
| 2.5 | Throwing inside the augmented step body propagates as the step's error — Inngest sees a failed step, no provider call leaves the process |
| 2.6 | `step.ai.infer`-shaped errors (model-side throws) are caught, `commitEstimated(outcome="PROVIDER_ERROR")` fires, THEN the error is re-thrown |
| 2.7 | `step.ai.wrap` works identically to `step.ai.infer` for reserve + commit (the only difference is the body source) |
| 2.8 | Adapter does NOT swallow Inngest's own `NonRetriableError` or similar control-flow errors |

## 3. Reserve / commit semantics (P0 — blocker)

| Check | Pass condition |
|---|---|
| 3.1 | `client.reserve` is called with `trigger=LLM_CALL_PRE` |
| 3.2 | `llmCallId = step.id` — exact equality with the Inngest step ID |
| 3.3 | `stepId = step.id` — one-to-one with the Inngest step ID |
| 3.4 | `decisionId = deriveUuidFromSignature(seed, { scope: "decision_id" })` where `seed = inngestIdempotencyKey ?? step.id` |
| 3.5 | `idempotencyKey = deriveIdempotencyKey({tenantId, sessionId, runId, stepId, llmCallId, trigger: "LLM_CALL_PRE"})` — attempt-invariant |
| 3.6 | `projectedClaims` come from `claimEstimator(input)` — invoked exactly once per reserve |
| 3.7 | `claimEstimate` (optional) is forwarded verbatim |
| 3.8 | On success, `commitEstimated` is called with `outcome="SUCCESS"` and `estimatedAmountAtomic` from `extractTotalTokens(result)` |
| 3.9 | On provider error, `commitEstimated` is called with `outcome="PROVIDER_ERROR"` and `estimatedAmountAtomic="0"` |
| 3.10 | If `commitEstimated` itself fails after a provider error, the original provider error wins — commit failure is logged but not re-thrown |
| 3.11 | `route` defaults to `"llm.call.inngest"`; consumer-provided value propagates |

## 4. Retry-dedup contract (P0 — headline)

| Check | Pass condition |
|---|---|
| 4.1 | `deriveIdentity` is deterministic AND attempt-invariant — same `(stepId, inngestIdempotencyKey, runId)` produces same `idempotencyKey` regardless of `attempt` |
| 4.2 | `deriveIdentity` prefers `inngestIdempotencyKey` over `stepId` as the seed when both present |
| 4.3 | `retryDedup.test.ts` R-03 passes: 3 attempts on the same step → exactly 1 sidecar `reserve` round-trip |
| 4.4 | Demo gate A5.5 passes against the real sidecar: `SPENDGUARD_DEMO_RETRIES=2` yields 1 `LLM_CALL_PRE` audit row |
| 4.5 | A NEW Inngest function invocation (new `ctx.runId`) for the same step name produces a DIFFERENT `idempotencyKey` — fresh runs are NOT deduped against prior runs |
| 4.6 | The retry-dedup contract is called out explicitly in JSDoc on `wrapWithSpendGuard` AND in README.md |

Drift on any of these breaks the no-double-billing-on-retry promise. P0 — blocker. This is the most important class of finding for D29.

## 5. Error propagation (P0)

| Check | Pass condition |
|---|---|
| 5.1 | `DecisionStopped` thrown from `reserve` propagates out of `sgStep.infer` without being swallowed |
| 5.2 | `DecisionDenied`, `DecisionSkipped`, `SidecarUnavailable` all propagate identically |
| 5.3 | `ApprovalRequired` without `onApprovalRequired` propagates |
| 5.4 | `ApprovalRequired` with `onApprovalRequired` returning a `DecisionOutcome` resumes — the resumed outcome drives the commit |
| 5.5 | `ApprovalRequired` with `onApprovalRequired` returning null/undef propagates the original error |
| 5.6 | A throw in `claimEstimator` propagates — not silently swallowed |
| 5.7 | EE-02 (denied → 0 fetch calls) is real: the `fetch` spy records 0 calls when `reserve` throws |

## 6. Identity derivation (P1)

| Check | Pass condition |
|---|---|
| 6.1 | `identity.ts` `deriveIdentity` covered to 100 % statements + branches |
| 6.2 | Missing `inngestIdempotencyKey` correctly falls back to `stepId` |
| 6.3 | `decisionId` is a valid UUIDv7-shape string per RFC 9562 |
| 6.4 | `idempotencyKey` matches `sg-[0-9a-f]{32}` pattern |
| 6.5 | Identity function does NOT depend on `attempt` directly — verified by branch coverage |
| 6.6 | Identity function does NOT include `model` or `body` in the seed — content-stability across retries verified |

## 7. Token-usage extraction (P1)

| Check | Pass condition |
|---|---|
| 7.1 | Reads `result.usage.total_tokens` first (OpenAI shape) |
| 7.2 | Falls back to `result.usage_metadata.total_tokens` (Anthropic / Gemini shape) |
| 7.3 | Falls back to `result.response_metadata.token_usage.total_tokens` (legacy shape) |
| 7.4 | Returns `0` when none present — never throws |
| 7.5 | `providerEventId` reads `result.id` first, then `result.response_metadata.id`, else `""` |
| 7.6 | Robust to non-object `usage` field |

## 8. ESM-only + tree-shakeability (P1)

| Check | Pass condition |
|---|---|
| 8.1 | `"type": "module"` in `package.json` |
| 8.2 | No CJS build artefact in `dist/` |
| 8.3 | `"sideEffects": false` |
| 8.4 | `treeShaking.test.ts` confirms `dist/index.js` does NOT statically import `@grpc/grpc-js` |
| 8.5 | tsup config produces ESM-only output |
| 8.6 | Type-only imports from `@spendguard/sdk` use `import type` syntax |

## 9. Bundle-size budget (P1)

| Check | Pass condition |
|---|---|
| 9.1 | `pnpm run size` script wired into `prepack` |
| 9.2 | `dist/index.js` minified ≤ 35 KB |
| 9.3 | `dist/index.js` gzipped ≤ 10 KB |
| 9.4 | Budget breach is a build failure, not a warning |

## 10. Cross-language idempotency parity (P1)

| Check | Pass condition |
|---|---|
| 10.1 | `deriveIdempotencyKey` output for the same `(tenantId, sessionId, runId, stepId, llmCallId)` tuple is byte-identical to Python `derive_idempotency_key(...)` |
| 10.2 | The shared fixture `sdk/fixtures/cross-language/inngest_agent_kit_v1.json` is committed |
| 10.3 | The TS parity test consumes the fixture — vectors cover with/without `inngestIdempotencyKey` × attempt-0/1/N invariance |
| 10.4 | Identity derivation falls back to D05's `deriveIdempotencyKey`, not a re-implementation — verified by grep |

P1 here (not P0 like D04) because there's no Python Inngest adapter to cross-check against — but the substrate-level contract still holds.

## 11. Concurrency safety (P1)

| Check | Pass condition |
|---|---|
| 11.1 | Two concurrent `sgStep.infer` calls with different `step.id`s do not cross-correlate — verified by W-15 |
| 11.2 | The wrap maintains NO module-level mutable state — verified by grep + closure-only state |
| 11.3 | The wrap is reusable across multiple Inngest function invocations from one `wrapWithSpendGuard` call |
| 11.4 | If `wrapWithSpendGuard` is called once per function-instantiation (Inngest hot-path pattern), no leak occurs — closure-scoped state only |

## 12. Demo correctness (P1)

| Check | Pass condition |
|---|---|
| 12.1 | `examples/inngest-agent-kit/src/agent_real_inngest_agent_kit.ts` connects to the demo sidecar UDS and runs `step.ai.infer` |
| 12.2 | `make demo-up DEMO_MODE=agent_real_inngest_agent_kit` exits 0; sidecar logs one RequestDecision ack |
| 12.3 | The demo container has a Node 20 base image stage with `@spendguard/inngest-agent-kit`, `@inngest/agent-kit`, `inngest` installed at build time |
| 12.4 | `deploy/demo/demo/run_demo.py` dispatches `agent_real_inngest_agent_kit` to `subprocess.run(["node", ...])` |
| 12.5 | Denied-budget run produces 0 OpenAI HTTP requests (proven via fetch-log assertion) |
| 12.6 | Audit row ordering: `LLM_CALL_PRE` `created_at` < first OpenAI request timestamp |
| 12.7 | Retry-dedup demo (A5.5): `SPENDGUARD_DEMO_RETRIES=2` yields exactly 1 `LLM_CALL_PRE` audit row across 3 attempts — proven against the REAL sidecar, not a mock |
| 12.8 | OPENAI_API_KEY missing → demo aborts with a clear error |

## 13. Documentation completeness (P2)

| Check | Pass condition |
|---|---|
| 13.1 | `sdk/typescript/integrations/inngest-agent-kit/README.md` 30-line quickstart works as-is |
| 13.2 | README includes a "Retry dedup" section explaining the contract in 1-2 paragraphs |
| 13.3 | Every public function has JSDoc with `@throws` block enumerating typed exceptions |
| 13.4 | `CHANGELOG.md` 0.1.0 entry calls out: "SpendGuard wrap for Inngest AgentKit `step.ai`; retry-safe via Inngest step identity reuse" |
| 13.5 | `LICENSE_NOTICES.md` lists `@inngest/agent-kit` (Apache-2.0), `inngest` (Apache-2.0), `@spendguard/sdk` (Apache-2.0) |
| 13.6 | `docs/site/docs/integrations/inngest-agent-kit.md` new page with retry-dedup section, demo command, denied-budget behaviour |
| 13.7 | `README.md` (repo root) `## Adapter integrations` table has the inngest-agent-kit row |

## 14. Security (P1)

| Check | Pass condition |
|---|---|
| 14.1 | No `eval`, `new Function`, or `Function.prototype.constructor` anywhere |
| 14.2 | Wrap never logs prompts at INFO level — only at TRACE (and only when explicitly configured) |
| 14.3 | `model` and `body` from the user are NOT deep-cloned into logs (could leak PII) |
| 14.4 | `step.id`, `runId`, `eventId` treated as opaque — no parsing, no eval |
| 14.5 | `claimEstimator` is called with a frozen-by-convention input object — adapter does not mutate user-provided structures |
| 14.6 | `inngestIdempotencyKey` is treated as opaque — adapter does not log it at INFO (it can encode PII in the user's customer code) |
| 14.7 | `npm audit --omit=dev` reports 0 high/critical advisories at publish time |

## 15. Publish pipeline (P1)

| Check | Pass condition |
|---|---|
| 15.1 | `.github/workflows/sdk-ts-inngest-agent-kit-publish.yml` exists |
| 15.2 | Triggered on `release` event + `workflow_dispatch` |
| 15.3 | `if: startsWith(github.ref, 'refs/tags/inngest-agent-kit-ts-v')` gates the publish job |
| 15.4 | `permissions: id-token: write` set on the publish job (OIDC) |
| 15.5 | `npm publish --provenance --access public` |
| 15.6 | Workflow runs lint, typecheck, test, build, size before publish |
| 15.7 | Lockfile-frozen install (`pnpm install --frozen-lockfile`) |

## 16. Slice-specific anti-scope

| Slice | Anti-scope check |
|---|---|
| `COV_D29_01_pkg_init` | No source files beyond placeholder `src/index.ts`; no tests beyond sanity import |
| `COV_D29_02_wrap_factory` | Factory + types only — no real client call yet; reserve/commit stubbed |
| `COV_D29_03_reserve_commit_retry_dedup` | No demo script; no docs page |
| `COV_D29_04_tests_mock_agent_kit` | Test helpers + tests only; no source changes |
| `COV_D29_05_demo_agent_real_inngest_agent_kit` | Demo script + Makefile + run_demo.py dispatch only; no `src/` changes |
| `COV_D29_06_docs_publish` | No source changes; only README, CHANGELOG, LICENSE_NOTICES, docs site page, repo-root adapter table, publish workflow |

## 17. Findings categorisation

| Category | Definition | R1 action |
|---|---|---|
| **P0** | Public-surface drift, throw propagation broken, retry-dedup contract broken, cross-language idempotency drift, security finding | Block. Fix before re-run. |
| **P1** | Spec gate failure, missing test, missing documentation, wrong error class | Block. Fix before re-run. |
| **P2** | Stylistic, minor JSDoc gap, non-critical perf, polish | Track as residual; may merge with note. |
| **Polish** | Naming preferences, comment wording | Track as residual; do not block. |

## 18. R1-R5 escalation rules

- Same finding in two consecutive rounds without progress → Staff+ panel arbitration per build-plan §1.3.
- Any §4 (retry-dedup) finding open at R3 → automatic Staff+ panel arbitration (the headline contract gets extra scrutiny).
- Any other P0 finding open at R5 → automatic Staff+ panel arbitration.
- Deferred P2/Polish residuals filed as `gh issue` referenced from the slice doc.

## 19. Residual triage template

```
Title: [D29 residual] <one-line summary>

Body:
- Slice: COV_D29_<NN>_<short>
- Round: R<n>
- Category: P<0|1|2>|Polish
- Spec ref: design.md §<n>, tests.md §<n>, acceptance.md §<n>
- Repro: <minimal command sequence>
- Why deferred: <one line>
- Suggested follow-up slice: <name or "TBD post-D29">
```

## 20. Sign-off

The reviewer signs off only when:

- Every P0 + P1 in §1–§15 is green.
- §4 retry-dedup contract is fully verified (R-03, A3.2, A3.3, A5.5 all green).
- Slice-specific anti-scope in §16 is honored.
- All R<=5 findings are resolved or filed as residuals.
- Acceptance gates in `acceptance.md` §12 are green.

If any of those fail → slice does not pass R review → loop continues.
