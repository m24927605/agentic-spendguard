# D06 — Vercel AI SDK `wrapLanguageModel` middleware (covers Mastra)

**Status:** Spec — Tier 2 (`framework-coverage-build-plan-2026-06.md` §2.2). **Owner:** Frontend Developer. **Upstream:** D05 (`@spendguard/sdk`) — [`D05/design.md`](../D05_ts_sdk_substrate/design.md) §4 is the contract. **Transitive coverage:** Mastra Agents call `generateText`/`streamText` from `ai`, so one wrap covers both ecosystems.

## 1. Problem

Vercel AI SDK v5+ is the dominant TS-side LLM router. It exposes `wrapLanguageModel({ model, middleware })` with three `LanguageModelV2Middleware` hooks:

| Hook | When | What we do |
|---|---|---|
| `transformParams` | before generate/stream | RESERVE — `client.reserve(LLM_CALL_PRE)` |
| `wrapGenerate` | non-streaming | invoke inner; success → COMMIT; failure → RELEASE |
| `wrapStream` | streaming | invoke inner; commit after stream `finish`; release on error/cancel |

Strongest TS hook in the ecosystem — strongly typed, covers both paths, composes onto any `@ai-sdk/*` provider. Mastra Agents call `generateText`/`streamText` underneath so one middleware satisfies both. Closest Python analog: `pydantic_ai.py::SpendGuardModel` — same reserve → invoke → commit_or_release shape, but here we use the framework's first-class extension point instead of subclassing.

## 2. Goals

1. Ship `@spendguard/vercel-ai` v0.1.0 at `sdk/typescript-vercel-ai/`. Apache-2.0. Peer-deps: `@spendguard/sdk@^0.1` + `ai@^5`. Dev-deps on `@ai-sdk/openai`, `@ai-sdk/anthropic`, `@mastra/core`.
2. Single public factory `createSpendGuardMiddleware(opts): LanguageModelV2Middleware`.
3. Idempotent under SDK retries — identical params → identical `idempotencyKey` → sidecar dedup.
4. Streaming COMMIT fires after `finish` part, never before. Mid-stream cancel → RELEASE.
5. Mastra coverage = subpath alias + doc + integration test, not a separate package.
6. Demo modes `agent_real_vercel_ai_ts` + `_stream` + `agent_real_mastra` in Makefile.

## 3. Non-goals

- Vercel AI SDK v4 (EOL mid-2026). Tool-call gating (`TOOL_CALL_PRE`) — v0.2. Mastra `Workflow` step gating — separate adapter; D06 covers Mastra Agents only. Mastra-specific package — users import from `@spendguard/vercel-ai/mastra` alias. Replacing AI SDK's `experimental_telemetry` — both run in parallel.

## 4. Public surface

```ts
import { createSpendGuardMiddleware } from "@spendguard/vercel-ai";
import { wrapLanguageModel } from "ai";
import { openai } from "@ai-sdk/openai";

const middleware = createSpendGuardMiddleware({
  client, budgetId, windowInstanceId, unit, pricing,
  // optional: claimEstimator, callSignature, runIdProvider, route, providerEventIdExtractor
});
const model = wrapLanguageModel({ model: openai("gpt-4o-mini"), middleware });
const { text } = await generateText({ model, prompt: "Hello" });
```

Mastra subpath `@spendguard/vercel-ai/mastra` re-exports the factory as `createSpendGuardLanguageMiddleware` — function-reference alias only. Mastra users replace the import and the rest is identical.

## 5. Architecture

```
wrapLanguageModel({ model: openai(...), middleware: createSpendGuardMiddleware(...) })
        │
SpendGuardMiddleware
   ├── transformParams ─► deriveCallIdentity → client.reserve(LLM_CALL_PRE)
   │                       └─ stash DecisionOutcome on WeakMap<params,StashEntry>
   ├── wrapGenerate    ─► await doGenerate(); success → commit + confirm; fail → release
   └── wrapStream      ─► const inner = await doStream();
                           return { ...inner, stream: instrument(inner.stream) }
                            ├─ on `finish` part: commit + confirmPublish
                            └─ on stream error / cancel: release
```

Identity derivation mirrors `pydantic_ai.py::_derive_call_identity`: hash `(prompt, modelSettings)` via D05's `defaultCallSignature` → derive `idempotencyKey`, `stepId`, `llmCallId`, `traceDecisionId`. Retry with identical params yields identical IDs; sidecar cache collapses. Stash uses `WeakMap<LanguageModelV2CallOptions, StashEntry>` — the params reference is preserved across the three hooks by v5, giving O(1) GC-safe lookup with no global state.

## 6. Streaming semantics

`wrapStream` returns `{ stream, request, response, warnings }`. We replace `stream` with a `TransformStream` that (a) forwards every part downstream unmodified, (b) watches for the `finish` part carrying `usage`+`finishReason`, (c) on `finish` enqueues the part and asynchronously commits + confirms (does not block consumer), (d) on terminal error / consumer cancel fires `release(...)`.

Race guard: a single `terminal: bool` ensures exactly one of `onFinish` / `onError` fires. Commit-side failure (e.g. sidecar UNAVAILABLE post-finish) does NOT corrupt the stream — downstream consumer still sees `finish`. Sidecar TTL reconciles via the audit chain.

## 7. Slice plan

| # | Slice | Size |
|---|---|---|
| 1 | `COV_S06_01_d06_package_init` (package.json, tsconfig, tsup, biome, vitest) | S |
| 2 | `COV_S06_02_d06_factory_skeleton` (factory + validation + WeakMap stash) | S |
| 3 | `COV_S06_03_d06_transform_params_reserve` (`transformParams` + identity) | M |
| 4 | `COV_S06_04_d06_wrap_generate_commit` (`wrapGenerate` + commit/rollback) | M |
| 5 | `COV_S06_05_d06_wrap_stream_commit` (TransformStream instrumentation) | M |
| 6 | `COV_S06_06_d06_provider_tests` (openai + anthropic + recorded fixtures) | M |
| 7 | `COV_S06_07_d06_mastra_integration` (demo scripts, docs page, Makefile, README) | M |
| 8 | `COV_S06_08_d06_publish_pipeline` (OIDC workflow, CHANGELOG, LICENSE_NOTICES) | S |

8 slices, 4S + 4M. Hits build-plan §4 ratio.

## 8. Locked design decisions

1. One package covers both ecosystems. Mastra subpath is a function-reference alias only.
2. `createSpendGuardMiddleware` is canonical. No class-based API.
3. Streaming commit fires after `finish`, never on first byte. Cancel = release.
4. WeakMap stash keyed by params reference. No global state.
5. AI SDK v5+ only. No v4 back-compat shim.
6. OTel reuses D05's `otelTracer`. No middleware-local OTel.
7. Tool-call gating deferred to v0.2.
8. Provider usage flows through `CommitEstimated` (Python Stage 7 mode). ProviderReport is v0.2.
9. `runIdProvider` wins over `currentRunPlan()`. Neither → `SpendGuardConfigError` on first `transformParams` (fail-fast).
10. DEGRADE patches raise `MutationApplyFailed` in v0.1 (matches `pydantic_ai.py:599-602`).

## 9. Amendment 2026-06-10 (D38 Phase-0)

This section is APPENDED per [`D38_mastra/design.md`](../D38_mastra/design.md) §9.1 (LOCKED). The original sections are left byte-intact above the amendment (no history rewrite). The title's "(covers Mastra)" stays for historical traceability; this amendment paragraph is the authoritative scope statement.

(a) **Stale transitive-coverage rationale.** The §1/§3-era rationale "Mastra Agents call `generateText`/`streamText` from `ai` underneath" is stale: Mastra owns its own agent loop since v0.14.0 (Aug 2025).

(b) **Mastra coverage re-scoped.** D06's Mastra coverage is re-scoped to **explicit AI SDK `LanguageModel` instances** handed to Mastra (Mastra still consumes `doGenerate`/`doStream` model objects); the model-router string syntax has no `wrapLanguageModel` injection point and is covered by **D38** (`@spendguard/mastra`).

(c) **Subpath alias status.** The `@spendguard/vercel-ai/mastra` subpath alias remains published and functional for explicit-instance users; its docs gain a pointer to `@spendguard/mastra` as the recommended Mastra integration.

(d) **Locked decision #5 corrected.** Locked decision #5 ("AI SDK v5+ only. No v4 back-compat shim.") is corrected to match shipped reality — per D38 design §9.2: shipped 0.x targets the AI SDK v4 line (`LanguageModelV1Middleware`); v5 (`LanguageModelV2Middleware`) and v6 (`LanguageModelV3`) variants are the D06 follow-on. The `ai` peer-dep is accordingly tightened from the unbounded `>=4.0.0` to `">=4.0.0 <5"`, released as `@spendguard/vercel-ai` 0.2.0 with a CHANGELOG entry (D38 design §9.2 records the full justification for deviating from a `>=5.0.0 <7` tightening, which would advertise compatibility the artifact does not have).

Consequence recorded honestly: because Mastra 1.0 consumes `LanguageModelV2`/`V3` instances, D06's *explicit-instance Mastra* coverage is bounded by the v4 model shape until the follow-on ships — one more reason D38 (`@spendguard/mastra`) is the primary Mastra answer.
