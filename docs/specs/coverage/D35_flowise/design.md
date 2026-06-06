# D35 — Flowise custom node (`@spendguard/flowise-nodes`)

**Status:** Spec — Tier 3 (build plan `framework-coverage-build-plan-2026-06.md` §2.3).
**Parent strategy:** [`framework-coverage-2026-06.md`](../../../strategy/framework-coverage-2026-06.md), no-code platform coverage.
**Owner sub-agent:** Frontend Developer.
**Upstream contract:** [`D05_ts_sdk_substrate/design.md`](../D05_ts_sdk_substrate/design.md) §4 (SDK), [`D04_langchain_ts/design.md`](../D04_langchain_ts/design.md) §4 (LangChain TS handler).
**Relation to D33/D34:** D33/D34 are base-URL recipes (host sets `OPENAI_BASE_URL`). Flowise has per-node ChatModel config, so host-level override does NOT gate per-node calls — D35 must ship a real node.

## 1. Problem

Flowise (~40k stars, Apache-2.0 + commercial) is a visual no-code agent builder. Each canvas node is a TypeScript class implementing `INode` from `flowise-components`. A user drags any `BaseChatModel` node onto the canvas; the LLM HTTP call happens inside that node, invisible to the host. To gate Flowise we need a node that drops in once and wraps any ChatModel, intercepting every chat invocation. Flowise's `flowise-components` npm pattern (and `~/.flowise/nodes/` drop-in for self-hosted installs) is how the ecosystem extends the builder.

## 2. Goals

1. Publish `@spendguard/flowise-nodes` v0.1.0 npm package, Apache-2.0, in-tree at `sdk/typescript/integrations/flowise/`.
2. Public surface: one Flowise `INode` class `SpendGuardChatModelWrapper`. Canvas shows one input anchor (`chatModel: BaseChatModel`) and one output (`BaseChatModel`). Internally attaches D04's `SpendGuardCallbackHandler` to the wrapped model's `callbacks` array.
3. Drop-in install: `npm i @spendguard/flowise-nodes` into the Flowise install, or copy `dist/` to `~/.flowise/nodes/spendguard/`, restart, wrapper appears under `Spend Guard` category.
4. Demo mode `flowise_real`: compose stands up Flowise + sidecar + control-plane; pre-baked chatflow POSTed via `POST /api/v1/chatflows`; prediction runs end-to-end through `SpendGuardChatModelWrapper(inner=ChatOpenAI(...))`; deny variant returns 4xx with a SpendGuard reason code.
5. ESM-only, Node 20.10+; peer-deps `@spendguard/sdk@^0.1.0`, `@spendguard/langchain@^0.1.0`, `flowise-components@>=2.0.0`.
6. Integration tests against `flowiseai/flowise:2.x` via testcontainers.

## 3. Non-goals

- Wrapping non-ChatModel nodes (embeddings, vector stores, tools) — gated via the D02 egress proxy.
- Mid-flow gating between nodes — cross-node budget is the contract layer's job.
- Flowise Cloud — only self-hosted Flowise is supported.
- Flowise marketplace listing — out of our control; we ship npm + docs only.
- Per-provider SpendGuard nodes — one wrapper covers all providers via the `BaseChatModel` anchor.

## 4. Public surface — LOCKED

The canvas-builder sees exactly one node:

| Field | Value |
|---|---|
| `label` | `SpendGuard ChatModel Wrapper` |
| `name` | `spendGuardChatModelWrapper` |
| `version` | `1.0` |
| `type` | `BaseChatModel` |
| `icon` | `spendguard.svg` (bundled) |
| `category` | `Spend Guard` |
| `baseClasses` | `["BaseChatModel", "BaseLanguageModel"]` |
| Inputs (required) | `chatModel: BaseChatModel`, `tenantId: string`, `budgetId: string`, `windowInstanceId: string`, `unit: string` (default `usd_micros`) |
| Inputs (optional) | `sidecarUds: string` (env `SPENDGUARD_SIDECAR_UDS` fallback), `route: string` (default `llm.call`), `claimEstimatorJson: string` (JSON override) |
| Output | `BaseChatModel` — the inner `chatModel` (same reference) with `SpendGuardCallbackHandler` appended to its `callbacks` array |

The class skeleton — locked at design time so slice authors do not re-litigate field names — is in `implementation.md` §3.

## 5. Architecture

```
Flowise canvas
    │
    ▼
ChatOpenAI node ─────┐
                     ▼
   SpendGuardChatModelWrapper.init({ chatModel, tenantId, budgetId, ... })
                     │
                     ├── getOrCreateClient(sidecarUds, tenantId)
                     ├── new SpendGuardCallbackHandler({ client, budgetId, ... })
                     ├── chatModel.callbacks.push(handler)
                     └── return chatModel  ← SAME reference
                     │
                     ▼
Downstream chain node consumes wrapped chatModel as a normal BaseChatModel
                     │
                     ▼
At invoke: LangChain.js → handleChatModelStart → SpendGuardCallbackHandler → client.reserve
```

Key facts: returns the **same** `chatModel` reference, handler appended non-destructively (no subclassing, no proxy); `SpendGuardClient` cached in a module-level `Map` keyed by `(tenantId, sidecarUds)` so re-running `init()` per invocation does not re-open UDS; default `claimEstimator` charges `1_000_000` atomic units (= $1 USD-micros, conservative) so a no-code builder gets a working install; the `claimEstimatorJson` input (JSON parsed at init) overrides.

## 6. Locked design decisions

1. **One wrapper node, not per-provider nodes** — the canvas builder drags one wrapper onto any ChatModel.
2. **D04 reused verbatim** — `SpendGuardCallbackHandler` does all the work; D35 is glue + node manifest only.
3. **`SpendGuardClient` cached per `(tenantId, sidecarUds)`** — avoids per-call UDS reconnect.
4. **Default conservative $1 claim** — node-builders override via `claimEstimatorJson`. Documented as conservative-not-accurate.
5. **Demo mode is `flowise_real`** — distinct from D04's `agent_real_langchain_ts`; both coexist.
6. **Peer-deps not deps** — `@spendguard/sdk`, `@spendguard/langchain`, and `flowise-components` are all peerDeps. The consumer's Flowise install wins.
7. **Self-hosted Flowise only** — Flowise Cloud is documented as not supported.

## 7. Slice plan

| Slice | Title | Size |
|---|---|---|
| `COV_D35_01_pkg_init` | package.json, tsconfig, tsup, biome, vitest, peer-deps, INode skeleton, icon | S |
| `COV_D35_02_wrapper_node` | `SpendGuardChatModelWrapper` class, `init()` mutates `callbacks` and returns same ref, client cache | M |
| `COV_D35_03_reserve_commit_wiring` | `claimEstimatorJson` parse, route input, throw-on-deny, unit tests vs mock sidecar | M |
| `COV_D35_04_tests_flowise_container` | testcontainers vs `flowiseai/flowise:2.x`, POST chatflow, assert reserve + commit, deny → 4xx | M |
| `COV_D35_05_demo_flowise_real` | `examples/flowise/`, compose service, `run_demo.py` dispatch, deny variant | M |
| `COV_D35_06_docs_publish` | docs page, README row, npm OIDC publish workflow | S |

Total: 6 slices, 2 S + 4 M. Acceptance in `acceptance.md`.
