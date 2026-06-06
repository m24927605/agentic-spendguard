# D29 — Acceptance Gates

Gates a reviewer (`superpowers:code-reviewer`) re-runs to confirm D29 is shipped. Every gate is runnable in the current repo state per build-plan §3. No gate depends on a third-party action SpendGuard cannot trigger.

## 1. Build + lint + typecheck

| Gate | Command (run from `sdk/typescript/integrations/inngest-agent-kit/`) | Pass condition |
|---|---|---|
| A1.1 | `pnpm install --frozen-lockfile` | exit 0; lockfile drift = fail |
| A1.2 | `pnpm run lint` | biome zero diagnostics |
| A1.3 | `pnpm run typecheck` | `tsc --noEmit` exit 0 |
| A1.4 | `pnpm run build` | tsup produces `dist/index.js` + `dist/index.d.ts`; exit 0 |
| A1.5 | `pnpm run size` | `dist/index.js` minified ≤ 35 KB; gzipped ≤ 10 KB |
| A1.6 | `pnpm exec tsc --noEmit -p tsconfig.json --traceResolution \| grep '@grpc/grpc-js'` | empty — adapter never resolves grpc directly |

## 2. Test gates

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | `pnpm run test` | vitest exit 0; coverage ≥ 92 % stmt, ≥ 88 % branch |
| A2.2 | `pnpm run test tests/wrap.test.ts` | W-01..W-17 all pass |
| A2.3 | `pnpm run test tests/retryDedup.test.ts` | R-01..R-08 all pass |
| A2.4 | `pnpm run test tests/errors.test.ts` | E-01..E-10 all pass; throw propagation proven |
| A2.5 | `pnpm run test tests/identity.test.ts` | I-01..I-07 all pass |
| A2.6 | `pnpm run test tests/extract.test.ts` | X-01..X-08 all pass |
| A2.7 | `pnpm run test tests/treeShaking.test.ts` | T-01..T-03 all pass |
| A2.8 | `pnpm run test tests/e2e/inngestDev.test.ts` | EE-01..EE-05 all pass; reserve-before-fetch ordering proven |
| A2.9 | `pnpm run test tests/idempotencyParity.test.ts` | CL-01 passes; keys match fixture |

## 3. Retry-dedup invariant (P0 — the headline contract)

| Gate | Path | Pass condition |
|---|---|---|
| A3.1 | `sdk/fixtures/cross-language/inngest_agent_kit_v1.json` | File exists, committed; ≥ 8 vectors covering with/without `inngestIdempotencyKey` × attempts 0 / 1 / N |
| A3.2 | `pnpm run test tests/retryDedup.test.ts` | R-01..R-08 pass; R-03 specifically proves one reserve + one commit across 3 attempts |
| A3.3 | `pnpm run test tests/e2e/inngestDev.test.ts -t "retry"` | EE-04 passes; in-memory dev runtime + 3 attempts → 1 sidecar reserve |
| A3.4 | Manual: pick 3 random fixture entries with different `inngestIdempotencyKey` / `step.id` shapes; recompute `deriveIdempotencyKey` by hand using Python `hashlib`-equivalent | Match |

A3 is the audit-chain invariant for the Inngest adapter. A regression breaks the no-double-billing-on-retry contract.

## 4. Public-surface gates

| Gate | Command | Pass condition |
|---|---|---|
| A4.1 | `node -e 'import("@spendguard/inngest-agent-kit").then(m => console.log(Object.keys(m).sort()))'` (after `pnpm pack && pnpm add <tarball>`) | Output is exactly `["wrapWithSpendGuard"]` plus TS type re-exports as types (`WrapOptions`, `ClaimEstimatorInput`, `ClaimEstimator`) |
| A4.2 | Sample consumer at `tests/_support/sampleConsumer.ts` does `wrapWithSpendGuard(stepAi, client, {...})` against a minimal options object | typecheck passes |
| A4.3 | `pnpm pack && tar -tzf spendguard-inngest-agent-kit-0.1.0.tgz \| grep -E "src/\|tests/\|node_modules"` | empty — only `dist/`, README, LICENSE, CHANGELOG ship |
| A4.4 | `tar -tzf spendguard-inngest-agent-kit-0.1.0.tgz \| grep -E "dist/index\.(js\|d\.ts)"` | both present |
| A4.5 | `wrapWithSpendGuard` return type's `infer` / `wrap` signatures match `@inngest/agent-kit@^0.1`'s `step.ai` exactly (compile-time gate via `expectAssignable`) | passes |

## 5. Demo gates (slice 5 — `agent_real_inngest_agent_kit`)

| Gate | Command | Pass condition |
|---|---|---|
| A5.1 | `make demo-up DEMO_MODE=agent_real_inngest_agent_kit` | exit 0; demo prints a result; sidecar logs one `RequestDecision` ack with `route=llm.call.inngest`, `trigger=LLM_CALL_PRE` |
| A5.2 | Post-run SQL: `SELECT count(*) FROM audit_outbox WHERE event_kind='LLM_CALL_PRE' AND tenant_id=...` | exactly 1 |
| A5.3 | Post-run SQL: `SELECT created_at FROM audit_outbox WHERE event_kind='LLM_CALL_PRE'` < first OpenAI HTTP request timestamp in `/tmp/openai-fetch-log.jsonl` | TRUE — proves pre-call gating |
| A5.4 | `make demo-up DEMO_MODE=agent_real_inngest_agent_kit SPENDGUARD_DEMO_DENY=1` | non-zero exit reflecting denied call; `/tmp/openai-fetch-log.jsonl` contains 0 OpenAI request lines |
| A5.5 | `make demo-up DEMO_MODE=agent_real_inngest_agent_kit SPENDGUARD_DEMO_RETRIES=2` (step body throws on attempts 0+1, succeeds on 2) | exit 0; SQL `SELECT count(*) FROM audit_outbox WHERE event_kind='LLM_CALL_PRE' AND tenant_id=...` returns **exactly 1**; sidecar logs at most 1 `RequestDecision` ack |
| A5.6 | `deploy/demo/compose.yml` includes a `demo-inngest-agent-kit` service with the right env vars + Node 20 base image | yaml lint passes |
| A5.7 | `deploy/demo/demo/run_demo.py` `DEMO_MODE == "agent_real_inngest_agent_kit"` dispatch branch present and tested via `pytest deploy/demo/demo/tests/test_run_demo.py -k inngest_agent_kit` | passes |

A5.5 is the production-grade proof of the retry-dedup contract — it runs against the real sidecar, not a mock.

## 6. Publish-pipeline dry-run

| Gate | Command | Pass condition |
|---|---|---|
| A6.1 | `pnpm pack` | Produces `spendguard-inngest-agent-kit-0.1.0.tgz` ≤ 50 KB tarball |
| A6.2 | `.github/workflows/sdk-ts-inngest-agent-kit-publish.yml` exists | references `sdk/typescript/integrations/inngest-agent-kit/` |
| A6.3 | Workflow includes `permissions: id-token: write` + `npm publish --provenance --access public` | OIDC trusted publisher |
| A6.4 | Workflow gated on `inngest-agent-kit-ts-v*` tag | tag prefix asserted |
| A6.5 | `gh workflow run sdk-ts-inngest-agent-kit-publish.yml --ref <branch>` reaches the publish step (skipped when not on a real release tag) | CI green for prior steps |

## 7. Runtime matrix

| Gate | Command | Pass condition |
|---|---|---|
| A7.1 | Node 20.10 CI shard `pnpm run test` | exit 0 |
| A7.2 | Node 22 LTS CI shard `pnpm run test` and `pnpm run test:e2e` | exit 0 |
| A7.3 | Bun 1.1+ CI shard `bun test tests/{wrap,identity,extract}.test.ts` | exit 0 (subset; advisory) |
| A7.4 | Deno 1.46+ CI shard same subset | exit 0 (advisory) |

A7.3/A7.4 advisory in v0.1.0; promoted to blocking in v0.2.

## 8. Documentation gates

| Gate | Path | Pass condition |
|---|---|---|
| A8.1 | `sdk/typescript/integrations/inngest-agent-kit/README.md` | Includes install + 30-line quickstart with `step.ai.infer` + `wrapWithSpendGuard` + a one-paragraph "Retry dedup" section explaining the contract |
| A8.2 | `sdk/typescript/integrations/inngest-agent-kit/CHANGELOG.md` | `0.1.0` entry calls out: "SpendGuard wrap for Inngest AgentKit `step.ai`; retry-safe via Inngest step identity reuse" |
| A8.3 | `sdk/typescript/integrations/inngest-agent-kit/LICENSE_NOTICES.md` | Lists `@inngest/agent-kit` (Apache-2.0), `inngest` (Apache-2.0), `@spendguard/sdk` (Apache-2.0) |
| A8.4 | `docs/site/docs/integrations/inngest-agent-kit.md` | New page; documents the wrap, the retry-dedup guarantee, demo command, denied-budget behaviour |
| A8.5 | `README.md` (repo root) `## Adapter integrations` table | Includes `@spendguard/inngest-agent-kit` row pointing to the npm page |

## 9. Compatibility with @inngest/agent-kit 0.1.x

| Gate | Command | Pass condition |
|---|---|---|
| A9.1 | Pin `@inngest/agent-kit@0.1.0` in devDeps, run full test suite | exit 0 |
| A9.2 | Bump devDep to `@inngest/agent-kit@0.1.<latest>`, run full test suite | exit 0 |
| A9.3 | `peerDependencies."@inngest/agent-kit": "^0.1.0"` | caret allows all 0.1.x |
| A9.4 | `peerDependencies."inngest": "*"` (the runtime SDK is a transitive peer the customer already owns) | passes audit |

A9 forces the adapter to track AgentKit's minor-version churn (still pre-1.0). Both bounds tested at every release; failure on a `0.1.<n>` release means we cut a compatibility patch within 14 days.

## 10. Substrate version gate (D05 contract)

| Gate | Command | Pass condition |
|---|---|---|
| A10.1 | `package.json` `peerDependencies."@spendguard/sdk"` | exactly `^0.1.0` (caret, not pin) |
| A10.2 | Tested against `@spendguard/sdk@0.1.0` | full suite green |
| A10.3 | `tests/_support/contractShape.ts` imports every D05 symbol the wrap uses (`SpendGuardClient`, `deriveIdempotencyKey`, `deriveUuidFromSignature`, `ApprovalRequired`, types) and typechecks | passes |
| A10.4 | If A10.3 ever fails: D05 has shipped a breaking change → cut a new D29 version with the bump | manual escalation per build-plan §1.4 |

## 11. Slice-level acceptance subset

| Slice | Subset acceptance |
|---|---|
| `COV_D29_01_pkg_init` | A1.1, A1.2, A1.3 pass; sanity import test passes |
| `COV_D29_02_wrap_factory` | W-09, W-13, W-14, W-16; I-06, I-07 pass |
| `COV_D29_03_reserve_commit_retry_dedup` | W-01..W-08, W-10..W-12, W-15, W-17; R-01..R-08; E-01..E-10; I-01..I-05; X-01..X-08 pass |
| `COV_D29_04_tests_mock_agent_kit` | EE-01..EE-05, CL-01 pass; A3.1, A3.2, A3.3 pass |
| `COV_D29_05_demo_agent_real_inngest_agent_kit` | A5.1..A5.7 pass — including the retry-dedup E2E (A5.5) |
| `COV_D29_06_docs_publish` | A1.5, A6.1..A6.5, A8.1..A8.5 pass |

## 12. Ship-readiness checklist

- [ ] Every gate in §1–§10 is green.
- [ ] `git status` shows no uncommitted files under `sdk/typescript/integrations/inngest-agent-kit/`.
- [ ] `git log --oneline sdk/typescript/integrations/inngest-agent-kit/` shows ≥ 6 atomic commits.
- [ ] `inngest-agent-kit-ts-v0.1.0` git tag exists on the slice-6 merge commit.
- [ ] Publish workflow was dry-run via `workflow_dispatch` on at least one PR; reached the npm publish step.
- [ ] `make demo-up DEMO_MODE=agent_real_inngest_agent_kit` green; SQL row + ordering check green.
- [ ] `make demo-up DEMO_MODE=agent_real_inngest_agent_kit SPENDGUARD_DEMO_DENY=1` proves denied-budget short-circuit (0 OpenAI requests).
- [ ] `make demo-up DEMO_MODE=agent_real_inngest_agent_kit SPENDGUARD_DEMO_RETRIES=2` proves retry-dedup (exactly 1 `LLM_CALL_PRE` row across 3 attempts).
- [ ] `README.md` (repo root) `## Adapter integrations` table has the inngest-agent-kit row.
- [ ] `docs/site/docs/integrations/inngest-agent-kit.md` shipped with a "Retry dedup" section.

When the checklist is fully green D29 is **shipped** per build-plan §7, and the `project_coverage_D29_shipped.md` memory entry is written per build-plan §8.
