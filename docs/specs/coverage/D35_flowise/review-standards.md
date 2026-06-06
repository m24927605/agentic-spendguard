# D35 — Review Standards

Use this checklist with `superpowers:code-reviewer` on every D35 slice. R1 runs the full checklist; R2-R5 focus on findings still open from the previous round. Findings are categorised P0 / P1 / P2 / Polish; P0 + P1 are blockers.

## 1. Public-surface lock (P0 — blocker)

The Flowise canvas-builder surface is what no-code users see. Drift after `design.md` is merged requires a re-spec.

| Check | Pass condition |
|---|---|
| 1.1 | `src/index.ts` exports `SpendGuardChatModelWrapper` — and ONLY this |
| 1.2 | The wrapper class sets `label = "SpendGuard ChatModel Wrapper"` (exact string) |
| 1.3 | `name === "spendGuardChatModelWrapper"` — used by Flowise as the slug; renames break installed chatflows |
| 1.4 | `type === "BaseChatModel"` — downstream chain nodes accept it |
| 1.5 | `category === "Spend Guard"` — locked side-panel grouping |
| 1.6 | `baseClasses` contains exactly `["BaseChatModel", "BaseLanguageModel"]` |
| 1.7 | `inputs` field-name set == `{chatModel, tenantId, budgetId, windowInstanceId, unit, sidecarUds, route, claimEstimatorJson}` — exact set, no additions, no removals |
| 1.8 | Required inputs (`chatModel`, `tenantId`, `budgetId`, `windowInstanceId`, `unit`) lack `optional: true`; optional inputs (`sidecarUds`, `route`, `claimEstimatorJson`) have `optional: true` |
| 1.9 | `module.exports = { nodeClass: SpendGuardChatModelWrapper }` present in the wrapper file (Flowise loader contract) |
| 1.10 | No `default export` on `src/index.ts` |
| 1.11 | Naming: camelCase on the public surface; no snake_case anywhere outside generated proto types |

If any of 1.1–1.11 fail → P0.

## 2. Flowise contract correctness (P0 — blocker)

| Check | Pass condition |
|---|---|
| 2.1 | The class implements `INode` from `flowise-components` — `import type { INode, INodeData, ICommonObject }` |
| 2.2 | `init(nodeData, _input, options)` signature matches Flowise 2.x exactly; return type is the inner `BaseChatModel` (NOT a Promise<unknown> opaque value lost downstream) |
| 2.3 | `init()` reads inputs from `nodeData.inputs?.<name>` — Flowise's documented contract — not from the second positional `_input` (which is the upstream node value, ignored here) |
| 2.4 | `flowise-components` is a peer dep, NOT a runtime dep — verified in `package.json` |
| 2.5 | Wrapped `chatModel` is returned by **reference identity**, not cloned (test W-01 proves this) |
| 2.6 | `chatModel.callbacks` is mutated **non-destructively** — existing callbacks are preserved (test W-03) |
| 2.7 | The icon file `spendguard.svg` is bundled in `dist/` (base64-embedded via tsup loader config) |
| 2.8 | The wrapper class has no `version` member typed as `string` — Flowise treats it as `number` per the 2.x INode contract |

## 3. Reserve / commit semantics (P0 — blocker)

| Check | Pass condition |
|---|---|
| 3.1 | A chat invocation on the returned model fires `client.reserve` with `trigger=LLM_CALL_PRE` (test W-11) |
| 3.2 | The `route` defaults to `"llm.call"`; consumer-provided `route` input wins (test W-14) |
| 3.3 | `idempotencyKey` derived through D04 — the wrapper does NOT compute its own (test P-01 — parity) |
| 3.4 | `projectedClaims` come from the `buildClaimEstimator(...)` function — invoked exactly once per reserve |
| 3.5 | Empty `claimEstimatorJson` → default $1_000_000 atomic units claim (test CE-01) |
| 3.6 | Non-empty `claimEstimatorJson` parses + overrides (test CE-03) |
| 3.7 | `DecisionStopped` thrown from sidecar propagates out of the wrapped `model.invoke()` (test W-15) — Flowise surfaces it as a 4xx on the prediction endpoint |
| 3.8 | `commitEstimated` fires on `handleLLMEnd` via the inherited D04 handler — no duplicate commit logic in D35 |

## 4. Cache correctness (P0 — blocker)

| Check | Pass condition |
|---|---|
| 4.1 | `getOrCreateClient` caches by `(tenantId, sidecarUds)` tuple — distinct tenants in the same install do NOT share a client (test W-16, C-04) |
| 4.2 | Distinct `sidecarUds` paths get distinct clients (test C-03) |
| 4.3 | Handshake failure does NOT cache a broken client — next call retries (test C-06) |
| 4.4 | Cache lives in module scope, not per-instance — re-running `init()` per Flowise invocation hits the cache (test C-02) |
| 4.5 | `_resetCacheForTests()` is exported only under a test-only path (jsdoc `@internal` + not in `src/index.ts` re-exports) |

## 5. Error propagation (P1)

| Check | Pass condition |
|---|---|
| 5.1 | Missing required input throws a `SpendGuardChatModelWrapper:` prefixed message (tests W-04..W-09) |
| 5.2 | `claimEstimatorJson` parse error throws with `not valid JSON` (test CE-05) |
| 5.3 | `claimEstimatorJson` missing `amountAtomic` throws with a clear message (test CE-04) |
| 5.4 | `SidecarUnavailable` from handshake propagates as-is (test C-06) |
| 5.5 | No `try { ... } catch { silently swallow }` anywhere in `nodes/SpendGuardChatModelWrapper.ts` (manual grep audit) |

## 6. Package hygiene (P1)

| Check | Pass condition |
|---|---|
| 6.1 | `package.json` `type === "module"` |
| 6.2 | `sideEffects: false` |
| 6.3 | `engines.node >= 20.10` |
| 6.4 | `peerDependencies` has `@spendguard/sdk@^0.1.0`, `@spendguard/langchain@^0.1.0`, `flowise-components@>=2.0.0` — NO regular `dependencies` block (or empty) |
| 6.5 | `files` array is `["dist/", "README.md", "LICENSE_NOTICES.md", "CHANGELOG.md"]` |
| 6.6 | `exports` map has only `.` with `types` + `import` keys; no CJS entry |
| 6.7 | No `default export` from any source file |
| 6.8 | Build output size: `dist/index.js` ≤ 50 KB minified, ≤ 16 KB gzipped (acceptance A1.5) |
| 6.9 | Tarball does NOT ship `src/`, `tests/`, `node_modules/` (acceptance A3.3) |

## 7. Demo (P1)

| Check | Pass condition |
|---|---|
| 7.1 | `deploy/demo/compose.yml` includes a `demo-flowise` service AND a `demo-flowise-runner` service |
| 7.2 | `examples/flowise/chatflow.json` is a valid Flowise 2.x chatflow with three nodes (`ChatOpenAI`, `SpendGuardChatModelWrapper`, `Conversation Chain`) and the wrapper wires the `chatModel` anchor into the chain |
| 7.3 | The demo runner script POSTs the chatflow, runs prediction, and asserts the sidecar audit_outbox got exactly one `LLM_CALL_PRE` row (acceptance A4.2) |
| 7.4 | Pre-call gating proven: `LLM_CALL_PRE` created_at < first OpenAI mock-server request timestamp (acceptance A4.3) |
| 7.5 | Deny variant returns 4xx or `STOP` body and the OpenAI mock-server receives zero requests (acceptance A4.4) |
| 7.6 | `DEMO_MODE == "flowise_real"` dispatch branch in `run_demo.py` tested |

## 8. Cross-language parity (P0)

| Check | Pass condition |
|---|---|
| 8.1 | `sdk/fixtures/cross-language/flowise_v1.json` exists, committed, ≥ 4 vectors |
| 8.2 | Parity test asserts the wrapper-routed call's `idempotencyKey` == a direct D04 handler attachment's `idempotencyKey` for identical inputs |
| 8.3 | The wrapper does NOT pass any custom `stepId` shape — it inherits D04's `${runId}:lc:${signature.slice(0,16)}` |

## 9. Manual reviewer checklist

When `superpowers:code-reviewer` runs, the human-eyeballable items the reviewer must spot-check:

- The `SpendGuardChatModelWrapper.ts` file is ≤ 200 LOC (the wrapper is intentionally thin glue).
- No `console.log` / `console.error` left behind.
- All `throw new Error(...)` messages prefix with `SpendGuardChatModelWrapper:` so they are greppable in Flowise logs.
- `module.exports` is the ONLY line in the file that uses CJS — it exists for Flowise's loader contract and is documented as such in a `// Flowise loader contract` comment immediately above.
- The README's three install paths are tested at least one against a real Flowise (the `~/.flowise/nodes/` drop-in is the path the E2E tests cover).
- The JSDoc on every exported member documents the no-code-builder context, not just the TS type.

## 10. R5 panel escalation triggers

If the reviewer escalates past R5, the Staff+ panel (per build-plan §1.3) should focus on:

- Whether the wrapper's same-reference return value will collide with future Flowise versions' chatflow caching (Flowise 2.x caches resolved node outputs; if a future major bump invalidates the assumption, the wrapper breaks silently).
- Whether the module-level client cache leaks across tenants if a Flowise install switches `SPENDGUARD_TENANT_ID` at runtime (currently per-tenant + per-uds → no, but worth a panel sanity-check).
- Whether the no-code "$1 default" claim semantic is documented loudly enough to avoid a customer surprise when their first month's bill includes uncapped denies.
