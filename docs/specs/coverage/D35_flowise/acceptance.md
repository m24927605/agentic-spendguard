# D35 — Acceptance Gates

Gates a reviewer (`superpowers:code-reviewer`) re-runs to confirm D35 is shipped. Every gate must be runnable in the current repo state per build-plan §3. No gate depends on a third-party action SpendGuard cannot trigger.

## 1. Build + lint + typecheck

| Gate | Command (run from `sdk/typescript/integrations/flowise/`) | Pass condition |
|---|---|---|
| A1.1 | `pnpm install --frozen-lockfile` | exit 0; lockfile drift = fail |
| A1.2 | `pnpm run lint` | biome zero diagnostics |
| A1.3 | `pnpm run typecheck` | `tsc --noEmit` exit 0 |
| A1.4 | `pnpm run build` | tsup produces `dist/index.js` + `dist/index.d.ts` + `dist/nodes/SpendGuardChatModelWrapper.js`; exit 0 |
| A1.5 | `pnpm run size` | `dist/index.js` minified ≤ 50 KB; gzipped ≤ 16 KB |
| A1.6 | `pnpm exec tsc --noEmit -p tsconfig.json --traceResolution \| grep '@grpc/grpc-js'` | empty — the wrapper never resolves grpc directly (D05 substrate handles it) |
| A1.7 | `pnpm exec tsc --noEmit -p tsconfig.json --traceResolution \| grep 'flowise-components'` | resolves to the peer dep only — no transitive pull into the bundle |

## 2. Test gates

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | `pnpm run test` | vitest exit 0; coverage ≥ 90 % stmt, ≥ 85 % branch |
| A2.2 | `pnpm run test tests/wrapper.test.ts` | W-01..W-16 all pass |
| A2.3 | `pnpm run test tests/clientCache.test.ts` | C-01..C-06 all pass |
| A2.4 | `pnpm run test tests/claimEstimator.test.ts` | CE-01..CE-07 all pass |
| A2.5 | `pnpm run test tests/flowiseManifest.test.ts` | M-01..M-08 all pass — locks the canvas surface |
| A2.6 | `D35_E2E=1 pnpm run test tests/e2e/flowiseContainer.test.ts` | F-01..F-07 all pass; requires Docker; CI E2E shard only |
| A2.7 | `pnpm run test tests/_fixtures` (manifest schema) | the committed `chatflow_minimal.json` and `chatflow_deny.json` parse and match Flowise 2.x schema |

## 3. Public-surface gates

| Gate | Command | Pass condition |
|---|---|---|
| A3.1 | `node -e 'import("@spendguard/flowise-nodes").then(m => console.log(Object.keys(m).sort()))'` (after `pnpm pack && pnpm add <tarball>`) | Output is exactly `["SpendGuardChatModelWrapper"]` |
| A3.2 | `node -e 'import("@spendguard/flowise-nodes/dist/nodes/SpendGuardChatModelWrapper.js").then(m => console.log(typeof m.default?.nodeClass, typeof m.nodeClass))'` | At least one of the two resolves to `"function"` (Flowise loader contract) |
| A3.3 | `pnpm pack && tar -tzf spendguard-flowise-nodes-0.1.0.tgz \| grep -E "src/\|tests/\|node_modules"` | empty — only `dist/`, README, LICENSE, CHANGELOG ship |
| A3.4 | `tar -tzf spendguard-flowise-nodes-0.1.0.tgz \| grep -E "dist/index\.(js\|d\.ts)"` | both present |
| A3.5 | `tar -tzf spendguard-flowise-nodes-0.1.0.tgz \| grep -E "dist/nodes/SpendGuardChatModelWrapper\.js"` | present — Flowise's loader scans this path |

## 4. Demo gates (slice 5 — `flowise_real`)

| Gate | Command | Pass condition |
|---|---|---|
| A4.1 | `make demo-up DEMO_MODE=flowise_real` | exit 0; chatflow prediction returns 200 with chat completion shape; sidecar logs ONE `RequestDecision` with `route=llm.call`, `trigger=LLM_CALL_PRE` |
| A4.2 | Post-run SQL: `SELECT count(*) FROM audit_outbox WHERE event_kind='LLM_CALL_PRE' AND tenant_id=...` | exactly 1 |
| A4.3 | Post-run timestamp comparison: the `LLM_CALL_PRE` row's `created_at` < the mock OpenAI server's first request timestamp (recorded in the demo container's log file `/tmp/openai-fetch-log.jsonl`) | TRUE — proves pre-call gating |
| A4.4 | `make demo-up DEMO_MODE=flowise_real SPENDGUARD_DEMO_DENY=1` | prediction returns 4xx OR a body with `STOP` / `DecisionStopped`; `/tmp/openai-fetch-log.jsonl` contains 0 OpenAI request lines |
| A4.5 | `deploy/demo/compose.yml` includes a `demo-flowise` service that extends `flowiseai/flowise:2.x` AND a `demo-flowise-runner` service that runs `examples/flowise/run_flowise_real.ts` against the Flowise endpoint | yaml lint passes |
| A4.6 | `deploy/demo/demo/run_demo.py` `DEMO_MODE == "flowise_real"` dispatch branch present and tested via `pytest deploy/demo/demo/tests/test_run_demo.py -k flowise_real` | passes |
| A4.7 | `examples/flowise/chatflow.json` is committed and parses as Flowise 2.x chatflow shape (has `nodes`, `edges`, three node entries) | structural assertion |

## 5. Publish-pipeline dry-run

| Gate | Command | Pass condition |
|---|---|---|
| A5.1 | `.github/workflows/sdk-ts-flowise-publish.yml` exists, uses Trusted Publisher OIDC against npm, triggered on `flowise-nodes-v*` tags | yaml lint passes, gh-workflow-validator passes |
| A5.2 | `pnpm pack` succeeds; tarball is ≤ 250 KB | size check |
| A5.3 | `pnpm publish --dry-run --no-git-checks` | exit 0; provenance field present |

## 6. Documentation gates

| Gate | Path | Pass condition |
|---|---|---|
| A6.1 | `docs/site/docs/integrations/flowise.md` exists | covers all three install paths (npm into source, `~/.flowise/nodes/` drop-in, Docker layer); has a screenshot of the canvas with the wrapper |
| A6.2 | `README.md` `## 🔌 Adapter integrations` table has a row for `Flowise (no-code visual builder)` linking to `@spendguard/flowise-nodes` and the integration docs page | row present |
| A6.3 | `examples/flowise/README.md` exists | explains how to run the demo locally end-to-end |
| A6.4 | `CHANGELOG.md` in the package has a `## [0.1.0]` entry referencing the slice plan | structural |

## 7. Public-canvas regression gates

D35's value-add is that the canvas builder sees an unchanging set of inputs. These gates lock the canvas surface against silent drift.

| Gate | Command | Pass condition |
|---|---|---|
| A7.1 | `pnpm run test tests/flowiseManifest.test.ts` | M-01..M-08 — locked node `label`, `name`, `type`, `category`, `baseClasses`, `inputs` schema |
| A7.2 | A snapshot of the `inputs` array is committed at `tests/__snapshots__/flowiseManifest.test.ts.snap`; renaming or reordering an input fails a vitest snapshot test | snapshot present + matching |

## 8. Cross-language parity (inherited from D04)

| Gate | Path | Pass condition |
|---|---|---|
| A8.1 | `sdk/fixtures/cross-language/flowise_v1.json` | committed; ≥ 4 vectors covering chat with + without parentRunId, with + without claimEstimatorJson override |
| A8.2 | `pnpm run test tests/_parity/idempotencyParity.test.ts` | P-01 — wrapper-routed call produces SAME idempotencyKey as direct D04 handler attachment |

## 9. What "D35 shipped" means

All gates A1.* through A8.* pass green on a clean repo at `main`, including the E2E gate when run under `D35_E2E=1` with Docker available. The README row + docs page + Makefile demo entry exist per build-plan §7 definition-of-done. The npm publish dry-run is green and the publish workflow file is in place.
