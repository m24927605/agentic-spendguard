# D08 — OpenAI Agents SDK TypeScript adapter (`@spendguard/openai-agents`)

**Status:** Spec — Tier 2 (build plan `framework-coverage-build-plan-2026-06.md` §2.2).
**Build plan:** §2.2 / §2.4.
**Owner sub-agent:** Frontend Developer.
**Upstream substrate:** [D05 `@spendguard/sdk`](../D05_ts_sdk_substrate/design.md) — locked surface.
**Python sibling (behavioural reference):** `sdk/python/src/spendguard/integrations/openai_agents.py`.

## 1. Problem

`@openai/agents` (TypeScript) is OpenAI's canonical JS agent runtime. Its `Model` interface (`OpenAIChatCompletionsModel`, `OpenAIResponsesModel`) is structurally identical to the Python SDK, so the Python wrapper (subclass `Model`, bracket `getResponse()` with `reserve → call → commitEstimated`) ports cleanly. TS-native stacks have no SpendGuard insertion point today.

## 2. Goals

1. Publish `@spendguard/openai-agents` v0.1.0 (Apache-2.0) at `sdk/typescript/packages/openai-agents/`.
2. Mirror Python behaviour 1:1: DENY ⇒ inner `Model` NEVER invoked; CONTINUE ⇒ inner runs ⇒ `commitEstimated` with total_tokens.
3. Public factory `withSpendGuard(inner, opts)` + subclass form `SpendGuardAgentsModel`.
4. Shared `runContext` (`AsyncLocalStorage`) so D04/D06/D08/D29 multi-framework runs share one trace.
5. Stream pass-through; per-chunk gating out of scope (parity with Python POC).
6. Default `claimEstimator` derived from inner model name (parity with Python `_default_estimator`).
7. New demo mode `agent_real_openai_agents_ts` runs end-to-end with a real Agent + `OpenAIChatCompletionsModel`.

## 3. Non-goals

Per-chunk stream gating. `OpenAIResponsesModel`-specific features. Bundling D05 (peer dep). DEGRADE auto-apply. Vendoring `@openai/agents`. Browser.

## 4. Public surface — LOCKED

```ts
import { withSpendGuard, SpendGuardAgentsModel, runContext, currentRunContext,
  type SpendGuardModelOptions, type ClaimEstimator } from "@spendguard/openai-agents";

// Factory (primary) + subclass (secondary):
export function withSpendGuard<M extends Model>(inner: M, opts: SpendGuardModelOptions): Model;
export class SpendGuardAgentsModel implements Model {
  constructor(opts: SpendGuardModelOptions & { inner: Model });
}

export interface SpendGuardModelOptions {
  client: SpendGuardClient;          // from @spendguard/sdk; handshaken
  budgetId: string;
  windowInstanceId: string;
  unit: UnitRef;
  pricing: PricingFreeze;
  claimEstimator?: ClaimEstimator;   // default derived from inner.model when omitted
}
export type ClaimEstimator = (input: unknown) => BudgetClaim[];

export interface RunContext { readonly runId: string }
export function runContext<T>(ctx: RunContext, fn: () => Promise<T>): Promise<T>;
export function currentRunContext(): RunContext; // throws outside an active context
```

HOF form (TS5 decorators not used). Node `AsyncLocalStorage` keyed on `Symbol.for("@spendguard/run-context/v1")` so D04/D06/D08/D29 dedupe to one instance.

## 5. Behaviour contract

For each `getResponse(...)`:

1. Read `runId` from `currentRunContext()`; outside ⇒ throw.
2. `signature = blake2b16(repr(input) + "|" + (systemInstructions ?? ""))`.
3. `llmCallId`, `decisionId` from `deriveUuidFromSignature(signature, { scope })`. `stepId = "${runId}:oai-call:${sig.slice(0,16)}"`. `idempotencyKey` from D05 `deriveIdempotencyKey(...)` with `trigger="LLM_CALL_PRE"`.
4. `claims = (claimEstimator ?? defaultClaimEstimator)(input)`.
5. `outcome = await client.reserve({ trigger:"LLM_CALL_PRE", runId, stepId, llmCallId, decisionId, route:"llm.call", projectedClaims: claims, idempotencyKey })`.
6. DENY/STOP/SKIP/APPROVAL ⇒ D05 throws typed error ⇒ inner NEVER reached. `ApprovalRequired` propagates so callers can `.resume()`.
7. CONTINUE ⇒ `inner.getResponse(...same args)`; extract `usage.totalTokens` (number | string | undef → number).
8. If `outcome.reservationIds.length > 0` ⇒ `client.commitEstimated({ ..., estimatedAmountAtomic: String(totalTokens), outcome:"SUCCESS" })`.
9. Return inner response unchanged.

`streamResponse()`: pass-through. No PRE/POST in v0.1. POST_D08 follow-on.

## 6. Architecture

```
@spendguard/openai-agents (npm, ESM-only, Node 20+, peer deps only)
├── src/index.ts          re-exports
├── src/withSpendGuard.ts factory
├── src/model.ts          SpendGuardAgentsModel class
├── src/core.ts           shared bracketing
├── src/signature.ts      blake2b16 fingerprint
├── src/usage.ts          totalTokens extraction
├── src/runContext.ts     AsyncLocalStorage
└── src/defaultEstimator.ts inner.model → ClaimEstimator
```

Subpaths `./model`, `./run-context`. Toolchain inherits D05 (tsup, vitest, biome, pnpm). Publish via `ts-sdk-openai-agents-v*` tag → npm OIDC. New demo mode `agent_real_openai_agents_ts` in `deploy/demo/demo/run_demo.py` spawns `node examples/openai-agents-ts-composite/dist/demo.js --real`; `demo-up` depends on `demo-ts-build`; Dockerfile installs Node 20.

## 7. Locked design decisions

1. **Composition primary, subclass secondary.** Both share `core.ts`.
2. **Stream pass-through.** Per-chunk gating is POST_D08.
3. **`commitEstimated` only, no `ProviderReport`.** Locked at D05 §4.2.
4. **Shared `runContext` storage** via `Symbol.for("@spendguard/run-context/v1")` so D04/D06/D08/D29 dedupe to one `AsyncLocalStorage`. D05 v0.2 subsumes; until then each adapter ships an identical 12-line copy.
5. **Default `claimEstimator`** from `inner.model` — values byte-identical to Python `_default_estimator.MODEL_BASELINE_TOKENS`.
6. **Idempotency key** from `(tenant, session, run, step, llmCall, trigger)`. Cross-language fixture enforces byte parity.
7. **`@openai/agents` peer range `>=0.3 <1`.**
8. **Subpath exports** `./model`, `./run-context`.

## 8. Slice plan

| Slice | Scope | Size |
|---|---|---|
| `COV_S08_01_d08_package_init` | package.json, tsconfig, tsup/biome/vitest config, README stub, workspace registration | S |
| `COV_S08_02_d08_with_spendguard_factory` | `withSpendGuard` + `SpendGuardAgentsModel` + `core.ts` + `signature.ts` + `usage.ts` + `runContext.ts` + default estimator | M |
| `COV_S08_03_d08_tests_unit` | vitest suite (factory + class + ALLOW/DENY/STOP/APPROVAL) + cross-language fixture extension (`openai_agents` section in `sdk/fixtures/cross-language/v1.json`) | M |
| `COV_S08_04_d08_demo_mock_real` | `examples/openai-agents-ts-composite/`: `demo.ts` `--mock` + `--real`, README | M |
| `COV_S08_05_d08_demo_mode_wireup` | `run_demo.py` mode + subprocess helper; `Makefile demo-up` depends on TS build; Dockerfile Node 20 | S |
| `COV_S08_06_d08_docs_publish` | integration doc, repo README adapter row, CHANGELOG, npm OIDC workflow, LICENSE_NOTICES | S |

Total: **6 slices** (within build-plan §4.1 TS-adapter 4-6 band).

## 9. Cross-language parity gates

`signatureOf` / `deriveUuidFromSignature` / `deriveIdempotencyKey` outputs byte-identical to their Python counterparts (`_signature` / `derive_uuid_from_signature` / `derive_idempotency_key`). Enforced by extension of `sdk/fixtures/cross-language/v1.json` `openai_agents` section in S08_03. Drift breaks audit-chain dedup across Python ↔ TS deploys.

## 10. Open questions — locked

Composition primary; stream gating OUT of v0.1.x; default estimator ported from Python `_default_estimator`; run-context key `Symbol.for("@spendguard/run-context/v1")`; peer range `>=0.3 <1`; publish tag prefix `ts-sdk-openai-agents-v*`.
