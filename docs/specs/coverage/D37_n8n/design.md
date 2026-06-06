# D37 — n8n community node (`n8n-nodes-spendguard`)

**Status:** Spec — Tier 3 (build plan `framework-coverage-build-plan-2026-06.md` §2.3).
**Parent strategy:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md) — no-code platform coverage.
**Owner sub-agent:** Frontend Developer.
**Upstream contracts:** [`D04_langchain_ts/design.md`](../D04_langchain_ts/design.md) (callback handler reused verbatim), [`D05_ts_sdk_substrate/design.md`](../D05_ts_sdk_substrate/design.md) §4 (client, IDs, errors).

## 1. Problem

n8n (`n8n-io/n8n` ~191k stars, Fair-code Sustainable Use License) is the dominant self-hostable no-code automation surface. Its AI Agent node (`@n8n/n8n-nodes-langchain`) is built on LangChain.js and accepts an `ai_languageModel` sub-node connection (e.g. `lmChatOpenAi`, `lmChatAnthropic`). That sub-node sends prompts straight to the provider — no pre-call refusal, no per-workflow attribution.

D37 ships a community node that wraps the upstream `ai_languageModel` sub-node with SpendGuard reserve+commit by installing D04's callback handler on the wrapped model. From the workflow author's view: drop a sub-node between Chat Model and AI Agent.

## 2. Goals

1. Publish `n8n-nodes-spendguard` to npm under n8n's community-node convention (`n8n-nodes-` prefix, `n8n-community-node-package` keyword, `n8n` block).
2. Public surface: one node class `SpendGuardChatModel` with one input + one output of type `NodeConnectionType.AiLanguageModel`. No bespoke LLM call; forward the wrapped `BaseChatModel`.
3. Behaviour: when the AI Agent invokes the model, D04's `SpendGuardCallbackHandler` fires `LLM_CALL_PRE`; deny throws and the AI Agent fails with a typed `NodeApiError`.
4. SpendGuard credential `spendGuardApi`: tenant ID, sidecar UDS, budget ID, window instance ID, runtime kind.
5. Pairs with `n8n >= 1.50` self-hosted (`N8N_COMMUNITY_PACKAGES_ENABLED=true`). Cloud out of scope.
6. Demo mode `n8n_real`: docker-compose service with the package preinstalled, an importable workflow JSON, and a smoke runner.
7. Apache-2.0 licence.

## 3. Non-goals

- A bespoke LLM call (we forward the upstream Chat Model).
- A SpendGuard Tool node (`ai_tool` wrap) — contract layer's job.
- A SpendGuard Memory node (`ai_memory`).
- Workflow-level cross-step budget reservation (D05 `withRunPlan` via Code node is the documented escape hatch).
- n8n Verified Community Nodes registry submission — post-GA.
- Wrapping non-Chat `BaseLLM` models; `handleLLMStart` is wired for D04 parity but Chat Model is the documented path.

## 4. Public surface — LOCKED

Node identity (frozen): `name: spendGuardChatModel`, `displayName: SpendGuard Chat Model`, `group: ["transform"]`, `version: 1`, one `AiLanguageModel` input + one output, credential `spendGuardApi` required, `codex.categories: ["AI"]` / `subcategories.AI: ["Language Models"]`.

Node parameters: `budgetIdOverride`, `route` (default `"llm.call"`), `runIdSource` (`"executionId"` / `"nodeName"` / `"custom"`), `customRunId` (expression, shown only for `custom`), `claimAmountAtomic` (default `"1000000"`), `unit` (default `"usd_micros"`).

Credential `SpendGuardApi` properties in order: `tenantId` (required), `socketPath` (default `/var/run/spendguard/sidecar.sock`), `budgetId` (required), `windowInstanceId` (required), `runtimeKind` (default `"n8n"`). No `test` function — handshake is lazy.

## 5. Architecture

n8n's AI Agent calls `getInputConnectionData(NodeConnectionType.AiLanguageModel, 0)` to receive a `BaseChatModel` from its upstream `ai_languageModel` input. The SpendGuard node's `supplyData()` does the same one connector upstream, pushes D04's `SpendGuardCallbackHandler` onto `model.callbacks`, and returns the same model instance downstream. D04 owns reserve/commit; D37 owns wiring.

```
[lmChatAnthropic] --(ai_languageModel)--> [SpendGuard Chat Model] --(ai_languageModel)--> [AI Agent]
                                              │ supplyData():
                                              │   1. credentials + params
                                              │   2. lazy SpendGuardClient (singleton per credential)
                                              │   3. SpendGuardCallbackHandler from @spendguard/langchain
                                              │   4. model.callbacks.push(handler)
                                              │   5. return { response: model }
```

Run identity: `runId = executionId:nodeName` (or override), `sessionId = executionId`, `stepId = nodeName`.

Client lifecycle is per process, not per execution: a singleton client per credential is built on first `supplyData()` and reused; closed on `process.on('beforeExit')`.

## 6. Locked design decisions

1. **Callback handler over subclass.** D04's pattern; n8n's sub-node mechanism makes wrapping zero-touch.
2. **Return the upstream model verbatim.** No proxy, no shim — just `model.callbacks.push(handler)`.
3. **`executionId` is the `sessionId`** (via `this.getExecutionId()`).
4. **One credential per tenant.** Covers any number of SpendGuard nodes in the workflow.
5. **Self-hosted only.** n8n Cloud policy bans UDS / local FS; we don't work around it.
6. **SDK deps are runtime deps (not peer-deps) AND CJS (not ESM).** Two deviations from D04 driven by n8n's plugin loader: it doesn't walk peer-dep manifests and is CJS-only as of 1.50.
7. **Singleton client per credential per process.** Avoids handshake-per-execution overhead.
8. **Pin SDK deps to exact `0.1.0`, not caret.** `deriveIdempotencyKey` output must be byte-identical for any `n8n-nodes-spendguard@X.Y.Z` install; a transitive D05 minor bump would silently change semantics. SDK minor bumps force a coordinated D37 release.
9. **Demo mode `n8n_real`** — single mode covers happy + deny via env toggle.

## 7. Slice plan

| Slice | Title | Size |
|---|---|---|
| `COV_D37_01_pkg_init` | n8n-convention `package.json`, tsconfig, gulp build, eslint + biome, vitest | S |
| `COV_D37_02_node_credential` | `SpendGuardApi` credential + `SpendGuardChatModel` skeleton w/ `supplyData` stub | S |
| `COV_D37_03_reserve_commit_wiring` | singleton client + D04 handler injection + run-ID derivation + error mapping | M |
| `COV_D37_04_tests_selfhost` | vitest unit + e2e vs docker-compose self-hosted n8n + mock sidecar | M |
| `COV_D37_05_demo_n8n_real` | `demo-n8n` compose service + workflow JSON + run_demo.py dispatch + deny toggle | M |
| `COV_D37_06_docs_publish` | npm OIDC publish workflow, README, docs site page, adapter table | S |

Total: **6 slices**, 3 S + 3 M. Acceptance in `acceptance.md`.
