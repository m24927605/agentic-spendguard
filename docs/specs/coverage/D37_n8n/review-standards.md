# D37 — Review Standards

Use this checklist with `superpowers:code-reviewer` on every D37 slice. R1 runs the full checklist; R2-R5 focus on findings still open from the previous round. Findings are categorised P0 / P1 / P2 / Polish; P0 + P1 are blockers.

## 1. n8n community-node compliance (P0 — blocker)

The n8n loader is strict; mistakes here surface as silent node-not-found errors in production.

| Check | Pass condition |
|---|---|
| 1.1 | `package.json` `name` is `n8n-nodes-spendguard` (must start with `n8n-nodes-`) |
| 1.2 | `package.json` `keywords` array contains `n8n-community-node-package` |
| 1.3 | `package.json` `n8n.n8nNodesApiVersion === 1` |
| 1.4 | `package.json` `n8n.credentials[]` paths point into `dist/credentials/` and the referenced files exist post-build |
| 1.5 | `package.json` `n8n.nodes[]` paths point into `dist/nodes/.../*.node.js` and exist post-build |
| 1.6 | `package.json` `type` is `"commonjs"` — n8n loader does NOT support ESM nodes as of n8n 1.50 |
| 1.7 | `package.json` `engines.node` is `>=20.10` or higher |
| 1.8 | `.eslintrc.js` extends `@n8n_io/eslint-config-node` and uses `eslint-plugin-n8n-nodes-base` |
| 1.9 | The published tarball contains `dist/` but NOT `src/`, `tests/`, `node_modules/`, or `tsconfig.json` |

Any of 1.1–1.9 fail → P0. n8n's loader silently skips malformed nodes; we cannot ship a node that loads on dev machines but not in production.

## 2. Public-surface lock (P0 — blocker)

| Check | Pass condition |
|---|---|
| 2.1 | Exactly one node class — `SpendGuardChatModel` — implementing `INodeType` |
| 2.2 | Exactly one credential class — `SpendGuardApi` — implementing `ICredentialType` |
| 2.3 | Node `type` (internal name) is `spendGuardChatModel` |
| 2.4 | Node `displayName` is `"SpendGuard Chat Model"` |
| 2.5 | Node `version` is `1` (integer, no decimal) |
| 2.6 | Node `inputs[0].type === NodeConnectionType.AiLanguageModel` |
| 2.7 | Node `outputs[0].type === NodeConnectionType.AiLanguageModel` |
| 2.8 | Node `credentials[0]` is `{ name: "spendGuardApi", required: true }` |
| 2.9 | Credential `name === "spendGuardApi"` |
| 2.10 | Credential properties in order: `tenantId`, `socketPath`, `budgetId`, `windowInstanceId`, `runtimeKind` |
| 2.11 | The node `properties[]` listed in `design.md` §4 are present in the documented order |
| 2.12 | No additional node classes, credential classes, or `n8n.nodes[]` paths are added |
| 2.13 | `version` bumps follow n8n's versioned-node convention (new behaviour = new version key, never mutate `version: 1` in-place) |

Drift after `design.md` is merged requires a re-spec.

## 3. AI Language Model wiring (P0 — blocker)

| Check | Pass condition |
|---|---|
| 3.1 | `supplyData` calls `this.getInputConnectionData(NodeConnectionType.AiLanguageModel, itemIndex)` exactly once |
| 3.2 | `supplyData` throws (not returns) when the upstream model is `undefined` / `null` |
| 3.3 | The returned model is the SAME object reference as the upstream — no Proxy, no spread, no clone |
| 3.4 | `upstream.callbacks` is normalised to an array (`Array.isArray()` guard) before pushing |
| 3.5 | Duplicate-registration guard: if the same handler instance is already on `callbacks`, do not push again (N-05) |
| 3.6 | `response: <upstream>` is the only key on the returned `SupplyData` object |
| 3.7 | `SpendGuardCallbackHandler` is the ONLY callback the node adds — no logger callbacks, no custom callbacks |

## 4. Run identity correctness (P0 — blocker)

| Check | Pass condition |
|---|---|
| 4.1 | `sessionId = executionId` from `this.getExecutionId()` |
| 4.2 | `runId` for `runIdSource = "executionId"` is `${executionId}:${nodeName}` exactly (colon separator) |
| 4.3 | `stepId = nodeName` always |
| 4.4 | `customRunId` empty string → falls back to the `executionId` mode (RI-04) |
| 4.5 | `runId` propagates from `resolveRunIdentity` → D04 handler → mock sidecar without transformation (P-01) |
| 4.6 | `idempotencyKey` on the reserve request equals `deriveIdempotencyKey({tenantId, sessionId, runId, stepId, llmCallId: runId, trigger: "LLM_CALL_PRE"})` byte-for-byte (P-02) |

P-01 + P-02 are the cross-deliverable parity invariants. A regression here breaks dedup across the entire SpendGuard audit chain for n8n users.

## 5. Client pool semantics (P1)

| Check | Pass condition |
|---|---|
| 5.1 | The pool key includes ONLY `tenantId` and `socketPath` (CP-08, CP-09) |
| 5.2 | Concurrent `acquireClient` calls share an in-flight Promise (CP-06) |
| 5.3 | Bounded at 16 entries with FIFO eviction (CP-04) |
| 5.4 | Evicted clients have `close()` called (CP-04) |
| 5.5 | Failed handshake → pool entry deleted; retry from scratch (CP-05) |
| 5.6 | `process.on("beforeExit")` registered exactly once at module load — not per `acquireClient` call |
| 5.7 | `SpendGuardClient` is NOT closed at end of `supplyData` — singleton survives across executions (EE-08) |

## 6. Error mapping (P0)

| Check | Pass condition |
|---|---|
| 6.1 | `DecisionStopped` / `DecisionDenied` / `DecisionSkipped` → `NodeApiError(httpCode: "403")` (ER-01..ER-03) |
| 6.2 | `ApprovalRequired` → `NodeApiError(httpCode: "428")` with `approvalRequestId` in description (ER-04) |
| 6.3 | `SidecarUnavailable` → `NodeApiError(httpCode: "503")` (ER-05) |
| 6.4 | `HandshakeError` → `NodeApiError(httpCode: "502")` (ER-06) |
| 6.5 | Generic `Error` → `new NodeApiError(node, err)` fallthrough (ER-07) |
| 6.6 | `null` / `undefined` input does NOT crash the mapper (ER-08) |
| 6.7 | The `decisionId` (when present) appears in the `description` so users can correlate to the SpendGuard console |
| 6.8 | The prompt text NEVER appears in the `NodeApiError` message or description (A12.4) |

## 7. Dependency strategy (P1)

| Check | Pass condition |
|---|---|
| 7.1 | `@spendguard/sdk` is a `dependencies` entry — NOT a peer dep (n8n loader does not honor peer deps for community nodes) |
| 7.2 | `@spendguard/langchain` is a `dependencies` entry — NOT a peer dep |
| 7.3 | Both deps are EXACT-pinned (no caret, no tilde) |
| 7.4 | `n8n-workflow` is a `peerDependencies` entry with `"*"` range |
| 7.5 | `@langchain/core` is a `peerDependencies` entry with `"^0.3.0"` |
| 7.6 | `devDependencies` includes test versions of `n8n-workflow` and `@langchain/core` at the floor (1.50 / 0.3.0) |

## 8. Demo correctness (P1)

| Check | Pass condition |
|---|---|
| 8.1 | `examples/n8n/workflows/n8n_real.workflow.json` validates against n8n's workflow JSON schema |
| 8.2 | The demo workflow contains AI Agent + SpendGuard Chat Model + (Anthropic OR OpenAI) Chat Model wired correctly |
| 8.3 | `deploy/demo/compose.yml` has a `demo-n8n` service with a pinned n8n image SHA (not `latest`) |
| 8.4 | The compose service mounts the SpendGuard UDS read-only |
| 8.5 | The compose service sets `N8N_COMMUNITY_PACKAGES_ENABLED=true` |
| 8.6 | `deploy/demo/demo/run_demo.py` dispatches `DEMO_MODE == "n8n_real"` to the trigger script |
| 8.7 | Denied-budget run produces 0 provider HTTP requests and a `403` execution error |
| 8.8 | Audit row ordering: `LLM_CALL_PRE` `created_at` < first provider HTTP request timestamp (recorded in fetch-log) |
| 8.9 | OPENAI_API_KEY / ANTHROPIC_API_KEY missing → demo aborts with a clear error |

## 9. Documentation completeness (P2)

| Check | Pass condition |
|---|---|
| 9.1 | `README.md` includes `n8n npm install n8n-nodes-spendguard` install line |
| 9.2 | `README.md` includes a "Self-hosted only — n8n Cloud not supported" notice (A11.5) |
| 9.3 | `README.md` includes a 30-line worked example: AI Agent + SpendGuard wrapper + Chat Model |
| 9.4 | `CHANGELOG.md` `0.1.0` entry calls out: "n8n community node wrapping `ai_languageModel` sub-nodes via `@spendguard/langchain` (D04)" |
| 9.5 | `LICENSE_NOTICES.md` lists n8n-workflow (Sustainable Use License — NOT redistributed), `@langchain/core` (MIT), `@spendguard/sdk` (Apache-2.0), `@spendguard/langchain` (Apache-2.0) |
| 9.6 | `docs/site/docs/integrations/n8n.md` exists with worked example + deny screenshot + limitations section |
| 9.7 | `README.md` (repo root) `## Adapter integrations` table has the new row |

## 10. Security (P1)

| Check | Pass condition |
|---|---|
| 10.1 | No `eval`, `new Function`, or `Function.prototype.constructor` in `src/`, `nodes/`, or `credentials/` (A12.2) |
| 10.2 | No `console.*` calls in built output (A11.6) |
| 10.3 | Credential `socketPath` is NEVER logged at INFO; only at TRACE if explicitly enabled |
| 10.4 | Prompt text NEVER appears in n8n execution logs or NodeApiError messages |
| 10.5 | Credential fields are NOT serialised into workflow JSON exports (A12.5) |
| 10.6 | `npm audit --omit=dev` reports 0 high/critical advisories at publish time |
| 10.7 | The node does not register a global `process.on` handler beyond the single `beforeExit` in `clientPool.ts` |

## 11. Build pipeline (P1)

| Check | Pass condition |
|---|---|
| 11.1 | `pnpm run build` produces `dist/nodes/SpendGuardChatModel/SpendGuardChatModel.node.js` + `.node.json` + `spendguard.svg` |
| 11.2 | `pnpm run lint` runs BOTH `eslint nodes credentials package.json` AND `biome check src tests` |
| 11.3 | Both linters report zero diagnostics on a clean tree |
| 11.4 | `pnpm pack` tarball ≤ 200 KB |
| 11.5 | Tarball includes only `dist/`, `package.json`, `README.md`, `LICENSE_NOTICES.md`, `CHANGELOG.md` |

## 12. Publish pipeline (P1)

| Check | Pass condition |
|---|---|
| 12.1 | `.github/workflows/sdk-ts-n8n-publish.yml` exists |
| 12.2 | Triggered on `release` event + `workflow_dispatch` |
| 12.3 | `if: startsWith(github.ref, 'refs/tags/n8n-spendguard-v')` gates the publish job |
| 12.4 | `permissions: id-token: write` set on the publish job (OIDC) |
| 12.5 | `npm publish --provenance --access public` |
| 12.6 | Workflow runs lint, typecheck, test, build, size before publish |
| 12.7 | Lockfile-frozen install (`pnpm install --frozen-lockfile`) |

## 13. Compatibility (P1)

| Check | Pass condition |
|---|---|
| 13.1 | Full test suite passes against `n8n-workflow@1.50.0` (floor) |
| 13.2 | Full test suite passes against `n8n-workflow@1.55.0` (mid) |
| 13.3 | Full test suite passes against `n8n-workflow@latest` |
| 13.4 | `peerDependencies."n8n-workflow"` is `"*"` |
| 13.5 | `@langchain/core@0.3.0` (floor) → full suite green |
| 13.6 | `@langchain/core@0.3.<latest>` → full suite green |

## 14. Cross-deliverable contract (P0)

| Check | Pass condition |
|---|---|
| 14.1 | D04's `SpendGuardCallbackHandler` constructor accepts the `runIdOverride / sessionIdOverride / stepId` options D37 uses (else: file an additive issue against D04 v0.1.1 and pin to a development build) |
| 14.2 | D05's exported error classes (`DecisionDenied`, `DecisionStopped`, `DecisionSkipped`, `ApprovalRequired`, `SidecarUnavailable`, `HandshakeError`) match the names D37 imports |
| 14.3 | `tests/_support/contractShape.ts` typechecks against the exact-pinned versions of D04 and D05 |
| 14.4 | If D04 or D05 ships a breaking change after D37 v0.1.0, a coordinated D37 v0.1.x bump follows within 14 days |

## 15. Slice-specific anti-scope

| Slice | Anti-scope check |
|---|---|
| `COV_D37_01_pkg_init` | No node implementation beyond a placeholder class stub; no credential beyond schema skeleton; no tests beyond a single sanity import |
| `COV_D37_02_node_credential` | Node `supplyData` returns the upstream model verbatim with NO handler injection — that lands in slice 3 |
| `COV_D37_03_reserve_commit_wiring` | No demo workflow JSON; no compose service; no docs page |
| `COV_D37_04_tests_selfhost` | No source changes beyond test helpers and contractShape.ts; only tests added |
| `COV_D37_05_demo_n8n_real` | Demo workflow + compose + run_demo.py dispatch + trigger script only; no node source changes |
| `COV_D37_06_docs_publish` | No source changes; only README, CHANGELOG, LICENSE_NOTICES, docs site page, repo-root adapter table, publish workflow |

## 16. Findings categorisation

| Category | Definition | R1 action |
|---|---|---|
| **P0** | n8n loader incompatibility, public-surface drift, wiring contract broken, run-identity drift, cross-deliverable break, security finding | Block. Fix before re-run. |
| **P1** | Spec gate failure, missing test, missing documentation, wrong error class, compat regression | Block. Fix before re-run. |
| **P2** | Stylistic, minor JSDoc gap, non-critical perf, polish | Track as residual; may merge with note. |
| **Polish** | Naming preferences, comment wording | Track as residual; do not block. |

## 17. R1-R5 escalation rules

- Same finding in two consecutive rounds without progress → Staff+ panel arbitration per build-plan §1.3.
- P0 finding open at R5 → automatic Staff+ panel arbitration.
- Deferred P2/Polish residuals filed as `gh issue` referenced from the slice doc.

## 18. Residual triage template

```
Title: [D37 residual] <one-line summary>

Body:
- Slice: COV_D37_<NN>_<short>
- Round: R<n>
- Category: P<0|1|2>|Polish
- Spec ref: design.md §<n>, tests.md §<n>, acceptance.md §<n>
- Repro: <minimal command sequence>
- Why deferred: <one line>
- Suggested follow-up slice: <name or "TBD post-D37">
```

## 19. Sign-off

The reviewer signs off only when:
- Every P0 + P1 in §1–§14 is green.
- Slice-specific anti-scope in §15 is honoured.
- All R≤5 findings are resolved or filed as residuals.
- Acceptance gates in `acceptance.md` §14 are green.

If any of those fail → slice does not pass R review → loop continues.
