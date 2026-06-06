# D37 — Acceptance Gates

Gates a reviewer (`superpowers:code-reviewer`) re-runs to confirm D37 is shipped. Every gate must be runnable in the current repo state per build-plan §3. No gate depends on a third-party action SpendGuard cannot trigger.

## 1. Build + lint + typecheck

| Gate | Command (run from `sdk/typescript/integrations/n8n/`) | Pass condition |
|---|---|---|
| A1.1 | `pnpm install --frozen-lockfile` | exit 0; lockfile drift = fail |
| A1.2 | `pnpm run lint` | both eslint (`@n8n_io/eslint-config-node` + `eslint-plugin-n8n-nodes-base`) and biome report zero diagnostics |
| A1.3 | `pnpm run typecheck` | `tsc --noEmit` exit 0 |
| A1.4 | `pnpm run build` | tsc emits `dist/`; gulp copies `dist/nodes/SpendGuardChatModel/spendguard.svg`; exit 0 |
| A1.5 | `pnpm exec eslint nodes credentials --no-eslintrc -c node_modules/eslint-plugin-n8n-nodes-base/lib/recommended.js` | zero diagnostics — n8n's official linter is green |
| A1.6 | `pnpm pack` | tarball ≤ 200 KB; only `dist/`, `package.json`, `README.md`, `LICENSE_NOTICES.md`, `CHANGELOG.md` present |

## 2. Test gates

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | `pnpm run test` | vitest exit 0; coverage ≥ 90 % stmt, ≥ 85 % branch |
| A2.2 | `pnpm run test tests/node.test.ts` | N-01..N-16 all pass |
| A2.3 | `pnpm run test tests/credential.test.ts` | C-01..C-08 all pass |
| A2.4 | `pnpm run test tests/clientPool.test.ts` | CP-01..CP-09 all pass |
| A2.5 | `pnpm run test tests/runIdentity.test.ts` | RI-01..RI-07 all pass |
| A2.6 | `pnpm run test tests/errors.test.ts` | ER-01..ER-10 all pass |
| A2.7 | `pnpm run test tests/e2e/selfHostedN8n.test.ts` | EE-01..EE-10 all pass |

## 3. n8n public-surface gates

| Gate | Command / Path | Pass condition |
|---|---|---|
| A3.1 | `package.json` `name` | exactly `n8n-nodes-spendguard` |
| A3.2 | `package.json` `keywords` includes `n8n-community-node-package` | required by n8n's loader |
| A3.3 | `package.json` `n8n.n8nNodesApiVersion` | exactly `1` |
| A3.4 | `package.json` `n8n.credentials` references `dist/credentials/SpendGuardApi.credentials.js` | matches built path |
| A3.5 | `package.json` `n8n.nodes` references `dist/nodes/SpendGuardChatModel/SpendGuardChatModel.node.js` | matches built path |
| A3.6 | Node `type` (internal identifier) | exactly `spendGuardChatModel` |
| A3.7 | Node `version` | exactly `1` |
| A3.8 | Node `inputs[0].type` | `NodeConnectionType.AiLanguageModel` |
| A3.9 | Node `outputs[0].type` | `NodeConnectionType.AiLanguageModel` |
| A3.10 | Credential `name` | exactly `spendGuardApi` |
| A3.11 | `pnpm exec eslint-plugin-n8n-nodes-base check` (manual) | green — community-node rules satisfied |

## 4. Wire-correctness gates (the gating contract)

| Gate | Command | Pass condition |
|---|---|---|
| A4.1 | EE-03 + EE-04 (see `tests.md`) | One `Reserve(LLM_CALL_PRE)` and one `CommitEstimated(SUCCESS)` per successful workflow run |
| A4.2 | EE-05 + EE-06 | A denied reserve aborts the workflow with `NodeApiError(httpCode: "403")` and zero provider HTTP requests |
| A4.3 | EE-08 + EE-09 | Cross-execution singleton: handshake count stays at 1 across 2+ runs |
| A4.4 | P-01 + P-02 (see `tests.md` §5) | `runId`, `sessionId`, `stepId` round-trip; `idempotencyKey` byte-identical to D05's helper output |

These are the hard contract gates. Any of A4.* failing = deliverable broken.

## 5. Demo gates (slice 5 — `n8n_real`)

| Gate | Command | Pass condition |
|---|---|---|
| A5.1 | `make demo-up DEMO_MODE=n8n_real` | exit 0; n8n container starts, workflow imports, smoke runner exits 0 |
| A5.2 | After A5.1: `psql -c "SELECT count(*) FROM audit_outbox WHERE event_kind='LLM_CALL_PRE' AND route='llm.call'"` | exactly 1 |
| A5.3 | After A5.1: `psql -c "SELECT event_kind FROM audit_outbox ORDER BY created_at"` | rows in order: `LLM_CALL_PRE`, `LLM_CALL_POST` |
| A5.4 | After A5.1: created_at of `LLM_CALL_PRE` < first Anthropic HTTP request timestamp recorded in `/tmp/anthropic-fetch-log.jsonl` | TRUE — pre-call gating |
| A5.5 | `make demo-up DEMO_MODE=n8n_real SPENDGUARD_DEMO_DENY=1` | non-zero exit; n8n execution log shows `NodeApiError(httpCode: "403")`; 0 Anthropic HTTP requests in `/tmp/anthropic-fetch-log.jsonl` |
| A5.6 | `deploy/demo/compose.yml` contains the `demo-n8n` service per `implementation.md` §8 | yaml lint passes; ports / volumes / env / image SHA all present |
| A5.7 | `deploy/demo/demo/run_demo.py` `DEMO_MODE == "n8n_real"` dispatch branch present | unit test `pytest deploy/demo/demo/tests/test_run_demo.py -k n8n_real` passes |
| A5.8 | `examples/n8n/workflows/n8n_real.workflow.json` validates against n8n's workflow JSON schema | `n8n import:workflow --input=n8n_real.workflow.json --dry-run` exit 0 |
| A5.9 | `examples/n8n/scripts/trigger_workflow.ts` runs against the running compose stack | exit 0 in happy path; non-zero in deny mode |

## 6. Publish-pipeline dry-run

| Gate | Command | Pass condition |
|---|---|---|
| A6.1 | `pnpm pack` | Produces `n8n-nodes-spendguard-0.1.0.tgz` ≤ 200 KB |
| A6.2 | `.github/workflows/sdk-ts-n8n-publish.yml` exists | references `sdk/typescript/integrations/n8n/` |
| A6.3 | Workflow includes `permissions: id-token: write` + `npm publish --provenance --access public` | OIDC trusted publisher with provenance |
| A6.4 | Workflow gated on `n8n-spendguard-v*` tag | tag prefix asserted |
| A6.5 | `gh workflow run sdk-ts-n8n-publish.yml --ref <branch>` reaches the publish step (skipped without a real release tag) | CI green for prior steps |
| A6.6 | Published tarball contents (after `pnpm pack && tar -tzf ...`) include `dist/nodes/SpendGuardChatModel/spendguard.svg` and `dist/nodes/SpendGuardChatModel/SpendGuardChatModel.node.json` | icon + codex metadata present in published artifact |

## 7. Runtime matrix

| Gate | Command | Pass condition |
|---|---|---|
| A7.1 | n8n 1.50.x CI shard `pnpm run test:e2e` | exit 0 |
| A7.2 | n8n 1.55.x CI shard `pnpm run test:e2e` | exit 0 |
| A7.3 | n8n latest-release CI shard `pnpm run test:e2e` | exit 0 |
| A7.4 | Node 20.10 CI shard `pnpm run test` | exit 0 |
| A7.5 | Node 22 LTS CI shard `pnpm run test` + `pnpm run test:e2e` | exit 0 |

## 8. Documentation gates

| Gate | Path | Pass condition |
|---|---|---|
| A8.1 | `sdk/typescript/integrations/n8n/README.md` | Includes (a) `n8n npm install n8n-nodes-spendguard` install line, (b) screenshot of the wired sub-node, (c) 30-line workflow JSON snippet with AI Agent + SpendGuard wrapper |
| A8.2 | `sdk/typescript/integrations/n8n/CHANGELOG.md` | `0.1.0` entry calls out: "n8n community node wrapping `ai_languageModel` sub-nodes via `@spendguard/langchain` (D04)" |
| A8.3 | `sdk/typescript/integrations/n8n/LICENSE_NOTICES.md` | Lists `n8n-workflow` (Sustainable Use License — peer-dep, NOT redistributed), `@langchain/core` (MIT), `@spendguard/sdk` (Apache-2.0), `@spendguard/langchain` (Apache-2.0) |
| A8.4 | `docs/site/docs/integrations/n8n.md` | Exists with install + worked example + deny screenshot + known-limitations section (cloud, multi-tool gating, memory wrapping) |
| A8.5 | `README.md` (repo root) `## Adapter integrations` table | Includes `n8n-nodes-spendguard` row pointing to the npm page + docs site link |
| A8.6 | `Makefile` has `demo-up DEMO_MODE=n8n_real` target reachable | `make -n demo-up DEMO_MODE=n8n_real` resolves without error |

## 9. Compatibility gates

| Gate | Command | Pass condition |
|---|---|---|
| A9.1 | Pin `n8n-workflow@1.50.0` in devDeps, run full test suite | exit 0 |
| A9.2 | Bump devDep to `n8n-workflow@1.55.0`, run full test suite | exit 0 |
| A9.3 | Bump devDep to `n8n-workflow@latest`, run full test suite | exit 0 |
| A9.4 | `peerDependencies."n8n-workflow"` is `"*"` | matches n8n community-node convention (n8n versions its workflow package per release) |
| A9.5 | `peerDependencies."@langchain/core"` is `"^0.3.0"` | matches D04 floor |

n8n minor-version churn is high; A9.* forces the deliverable to track. If A9.3 fails on a new n8n release we ship a compatibility patch within 14 days.

## 10. Upstream version gate (D04 + D05 contracts)

| Gate | Command | Pass condition |
|---|---|---|
| A10.1 | `package.json` `dependencies."@spendguard/sdk"` | exactly `0.1.0` (pinned, NOT caret) |
| A10.2 | `package.json` `dependencies."@spendguard/langchain"` | exactly `0.1.0` (pinned, NOT caret) |
| A10.3 | `tests/_support/contractShape.ts` imports every D04/D05 symbol the node uses (`SpendGuardCallbackHandler` from D04; `SpendGuardClient`, `DecisionDenied`, `DecisionStopped`, `DecisionSkipped`, `ApprovalRequired`, `SidecarUnavailable`, `HandshakeError` from D05) and typechecks | passes |
| A10.4 | If A10.3 fails: D04 or D05 has shipped a breaking change — cut a new D37 version with the bump | manual escalation, build-plan §1.4 |

## 11. n8n community-node submission requirements

| Gate | Path / Command | Pass condition |
|---|---|---|
| A11.1 | Repo contains a `LICENSE` file (Apache-2.0) at `sdk/typescript/integrations/n8n/` OR is covered by the repo-root LICENSE | one of them exists |
| A11.2 | `package.json` `author` is non-empty | n8n requires it |
| A11.3 | `package.json` `description` is between 20 and 200 chars | n8n requires it |
| A11.4 | Node icon present in BOTH 16×16 and 60×60 viewports (single SVG with `<symbol>` patterns OK) | `pnpm exec node scripts/verify-icon.js` exit 0 |
| A11.5 | `README.md` contains an explicit "Self-hosted only" notice | grep gate |
| A11.6 | No `console.log` / `console.debug` statements in `dist/` | `grep -r "console\." dist/` returns empty |

These gates are derived from n8n's published community-node guidelines. They are NOT gates for the verified-registry (that's a separate workstream); they are gates for "this node loads cleanly on a self-hosted n8n with `N8N_COMMUNITY_PACKAGES_ENABLED=true`".

## 12. Security gates

| Gate | Command | Pass condition |
|---|---|---|
| A12.1 | `npm audit --omit=dev` | zero high / critical advisories |
| A12.2 | `grep -rE "eval\(\|new Function\(" src nodes credentials` | empty |
| A12.3 | The credential class does NOT log the resolved socketPath at INFO | grep against build output |
| A12.4 | E2E test EE-05 produces an `n8n.execution` log entry with the SpendGuard `decisionId` but NOT the prompt text | privacy invariant |
| A12.5 | No credential field is serialised into the workflow JSON export | manual: import then `n8n export:workflow` shows credential as `id`-only reference |

## 13. Slice-level acceptance subset

| Slice | Subset acceptance |
|---|---|
| `COV_D37_01_pkg_init` | A1.1, A1.2, A1.3, A1.4 pass; minimal sanity import test passes |
| `COV_D37_02_node_credential` | A3.1..A3.11, C-01..C-08 pass; N-01..N-04 pass |
| `COV_D37_03_reserve_commit_wiring` | N-05..N-16, CP-01..CP-09, RI-01..RI-07, ER-01..ER-10 pass; A4.1..A4.4 pass against mock sidecar |
| `COV_D37_04_tests_selfhost` | EE-01..EE-10 pass; A2.1 coverage targets met; A7.1..A7.5 green |
| `COV_D37_05_demo_n8n_real` | A5.1..A5.9 pass |
| `COV_D37_06_docs_publish` | A1.6, A6.1..A6.6, A8.1..A8.6, A11.1..A11.6 pass |

## 14. Ship-readiness checklist

- [ ] Every gate in §1–§12 is green.
- [ ] `git status` shows no uncommitted files under `sdk/typescript/integrations/n8n/`.
- [ ] `git log --oneline sdk/typescript/integrations/n8n/` shows ≥ 6 atomic commits.
- [ ] `n8n-spendguard-v0.1.0` git tag exists on the slice-6 merge commit.
- [ ] Publish workflow was dry-run via `workflow_dispatch` on at least one PR; reached the npm publish step (skipped at the final guard).
- [ ] `make demo-up DEMO_MODE=n8n_real` green; SQL row + ordering checks both green.
- [ ] `make demo-up DEMO_MODE=n8n_real SPENDGUARD_DEMO_DENY=1` proves the denied-budget short-circuit (0 provider requests).
- [ ] `README.md` (repo root) `## Adapter integrations` table has the `n8n-nodes-spendguard` row.
- [ ] `docs/site/docs/integrations/n8n.md` exists and renders.

When the checklist is fully green D37 is **shipped** per build-plan §7 definition of done, and the `project_coverage_D37_shipped.md` memory entry is written per build-plan §8.
