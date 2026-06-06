# D04 — Acceptance Gates

Gates a reviewer (`superpowers:code-reviewer`) re-runs to confirm D04 is shipped. Every gate must be runnable in the current repo state per build-plan §3. No gate depends on a third-party action SpendGuard cannot trigger.

## 1. Build + lint + typecheck

| Gate | Command (run from `sdk/typescript/integrations/langchain/`) | Pass condition |
|---|---|---|
| A1.1 | `pnpm install --frozen-lockfile` | exit 0; lockfile drift = fail |
| A1.2 | `pnpm run lint` | biome zero diagnostics |
| A1.3 | `pnpm run typecheck` | `tsc --noEmit` exit 0 |
| A1.4 | `pnpm run build` | tsup produces `dist/index.js` + `dist/index.d.ts`; exit 0 |
| A1.5 | `pnpm run size` | `dist/index.js` minified ≤ 40 KB; gzipped ≤ 12 KB |
| A1.6 | `pnpm exec tsc --noEmit -p tsconfig.json --traceResolution \| grep '@grpc/grpc-js'` | empty — the adapter never resolves grpc directly (substrate handles it) |

## 2. Test gates

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | `pnpm run test` | vitest exit 0; coverage ≥ 90 % stmt, ≥ 85 % branch |
| A2.2 | `pnpm run test tests/handler.test.ts` | H-01..H-18 all pass |
| A2.3 | `pnpm run test tests/streaming.test.ts` | S-01..S-04 all pass |
| A2.4 | `pnpm run test tests/errors.test.ts` | E-01..E-10 all pass; throw propagation proven |
| A2.5 | `pnpm run test tests/extract.test.ts` | X-01..X-08 all pass |
| A2.6 | `pnpm run test tests/treeShaking.test.ts` | T-01..T-03 all pass |
| A2.7 | `pnpm run test tests/e2e/chatOpenAI.test.ts` | EE-01..EE-05 all pass; reserve-before-fetch ordering proven |
| A2.8 | `pnpm run test tests/idempotencyParity.test.ts` | CL-01 passes; key matches Python output |

## 3. Cross-language parity (P0)

| Gate | Path | Pass condition |
|---|---|---|
| A3.1 | `sdk/fixtures/cross-language/langchain_v1.json` | File exists, committed; ≥ 16 vectors covering chat + LLM, with + without parent run id |
| A3.2 | `pnpm run test tests/idempotencyParity.test.ts` + `make -C sdk/python test TEST=tests/integrations/test_langchain_idempotency_parity.py` | Both suites parse the same fixture; zero diffs |
| A3.3 | Manual: pick 3 random fixture entries; recompute `deriveIdempotencyKey` by hand using Python `hashlib` | Match |

This gate is the Python+TS audit-chain invariant for the LangChain adapter — a regression breaks dedup against the existing Python langchain users.

## 4. Public-surface gates

| Gate | Command | Pass condition |
|---|---|---|
| A4.1 | `node -e 'import("@spendguard/langchain").then(m => console.log(Object.keys(m).sort()))'` (after `pnpm pack && pnpm add <tarball>`) | Output is exactly `["SpendGuardCallbackHandler", "SpendGuardCallbackHandlerOptions"]` plus TS type re-exports as types |
| A4.2 | Sample consumer at `tests/_support/sampleConsumer.ts` does `new SpendGuardCallbackHandler({...})` against a minimal options object | typecheck passes |
| A4.3 | `pnpm pack && tar -tzf spendguard-langchain-0.1.0.tgz \| grep -E "src/\|tests/\|node_modules"` | empty — only `dist/`, README, LICENSE, CHANGELOG ship |
| A4.4 | `tar -tzf spendguard-langchain-0.1.0.tgz \| grep -E "dist/index\.(js\|d\.ts)"` | both present |

## 5. Demo gates (slice 5 — `agent_real_langchain_ts`)

| Gate | Command | Pass condition |
|---|---|---|
| A5.1 | `make demo-up DEMO_MODE=agent_real_langchain_ts` | exit 0; `ChatOpenAI reply:` printed; sidecar logs one `RequestDecision` ack with `route=llm.call`, `trigger=LLM_CALL_PRE` |
| A5.2 | Post-run SQL: `SELECT count(*) FROM audit_outbox WHERE event_kind='LLM_CALL_PRE' AND tenant_id=...` | exactly 1 |
| A5.3 | Post-run SQL: `SELECT created_at FROM audit_outbox WHERE event_kind='LLM_CALL_PRE'` < first OpenAI HTTP request timestamp (recorded in the demo container's log file `/tmp/openai-fetch-log.jsonl`) | TRUE — proves pre-call gating |
| A5.4 | `make demo-up DEMO_MODE=agent_real_langchain_ts SPENDGUARD_DEMO_DENY=1` | non-zero exit reflecting denied call; `/tmp/openai-fetch-log.jsonl` contains 0 OpenAI request lines |
| A5.5 | `deploy/demo/compose.yml` includes a `demo-langchain-ts` service with the right env vars + the node base image | yaml lint passes |
| A5.6 | `deploy/demo/demo/run_demo.py` `DEMO_MODE == "agent_real_langchain_ts"` dispatch branch present and tested via `pytest deploy/demo/demo/tests/test_run_demo.py -k langchain_ts` | passes |

## 6. Publish-pipeline dry-run

| Gate | Command | Pass condition |
|---|---|---|
| A6.1 | `pnpm pack` | Produces `spendguard-langchain-0.1.0.tgz` ≤ 60 KB tarball |
| A6.2 | `.github/workflows/sdk-ts-langchain-publish.yml` exists | references `sdk/typescript/integrations/langchain/` |
| A6.3 | Workflow includes `permissions: id-token: write` + `npm publish --provenance --access public` | OIDC trusted publisher |
| A6.4 | Workflow gated on `langchain-ts-v*` tag | tag prefix asserted |
| A6.5 | `gh workflow run sdk-ts-langchain-publish.yml --ref <branch>` reaches the publish step (skipped when not on a real release tag) | CI green for prior steps |

## 7. Runtime matrix

| Gate | Command | Pass condition |
|---|---|---|
| A7.1 | Node 20.10 CI shard `pnpm run test` | exit 0 |
| A7.2 | Node 22 LTS CI shard `pnpm run test` and `pnpm run test:e2e` | exit 0 |
| A7.3 | Bun 1.1+ CI shard `bun test tests/{handler,extract,inflight}.test.ts` | exit 0 (subset; advisory) |
| A7.4 | Deno 1.46+ CI shard same subset | exit 0 (advisory) |

A7.3/A7.4 advisory in v0.1.0; promoted to blocking in v0.2.

## 8. Documentation gates

| Gate | Path | Pass condition |
|---|---|---|
| A8.1 | `sdk/typescript/integrations/langchain/README.md` | Includes install + 30-line quickstart with `ChatOpenAI` + `SpendGuardCallbackHandler` |
| A8.2 | `sdk/typescript/integrations/langchain/CHANGELOG.md` | `0.1.0` entry calls out: "TS counterpart of `spendguard-sdk[langchain]` (Python) v0.5.1; callback-handler shape" |
| A8.3 | `sdk/typescript/integrations/langchain/LICENSE_NOTICES.md` | Lists `@langchain/core` (MIT), `@spendguard/sdk` notices |
| A8.4 | `docs/site/docs/integrations/langchain.md` | Updated with a TS variant section pointing to `@spendguard/langchain`; the Python section preserved |
| A8.5 | `README.md` (repo root) `## Adapter integrations` table | Includes `@spendguard/langchain` (TS) row pointing to the npm page |

## 9. Compatibility with @langchain/core 0.3.x

| Gate | Command | Pass condition |
|---|---|---|
| A9.1 | Pin `@langchain/core@0.3.0` in devDeps, run full test suite | exit 0 |
| A9.2 | Bump devDep to `@langchain/core@0.3.<latest>`, run full test suite | exit 0 |
| A9.3 | `peerDependencies."@langchain/core": "^0.3.0"` | caret allows all 0.3.x |

LangChain.js minor-version churn is high (≥ 6 minor releases in 0.3.x line as of 2026-06). This gate forces the adapter to track. Both bounds tested at every release; if A9.2 fails on a `0.3.<n>` release we ship a compatibility patch within 14 days.

## 10. Substrate version gate (D05 contract)

| Gate | Command | Pass condition |
|---|---|---|
| A10.1 | `package.json` `peerDependencies."@spendguard/sdk"` | exactly `^0.1.0` (caret, not pin) — minor bumps allowed under semver |
| A10.2 | Tested against `@spendguard/sdk@0.1.0` | full suite green |
| A10.3 | `tests/_support/contractShape.ts` imports every D05 symbol the handler uses (`SpendGuardClient`, `deriveIdempotencyKey`, `deriveUuidFromSignature`, `defaultCallSignature`, `ApprovalRequired`, types) and typechecks | passes |
| A10.4 | If A10.3 ever fails: D05 has shipped a breaking change → cut a new D04 version with the bump | manual escalation, build-plan §1.4 |

## 11. Slice-level acceptance subset

| Slice | Subset acceptance |
|---|---|
| `COV_D04_01_pkg_init` | A1.1, A1.2, A1.3 pass; sanity import test passes |
| `COV_D04_02_handler_skeleton` | IF-01..IF-04, H-17, H-18 pass |
| `COV_D04_03_reserve_commit_wiring` | H-01..H-16, E-01..E-10, X-01..X-08 pass |
| `COV_D04_04_tests_mock_sidecar` | S-01..S-04, EE-01..EE-05, CL-01, CL-02 pass; A3.1, A3.2 pass |
| `COV_D04_05_demo_agent_real_langchain_ts` | A5.1..A5.6 pass |
| `COV_D04_06_docs_publish` | A1.5, A6.1..A6.5, A8.1..A8.5 pass |

## 12. Ship-readiness checklist

- [ ] Every gate in §1–§10 is green.
- [ ] `git status` shows no uncommitted files under `sdk/typescript/integrations/langchain/`.
- [ ] `git log --oneline sdk/typescript/integrations/langchain/` shows ≥ 6 atomic commits.
- [ ] `langchain-ts-v0.1.0` git tag exists on the slice-6 merge commit.
- [ ] Publish workflow was dry-run via `workflow_dispatch` on at least one PR; reached the npm publish step.
- [ ] `make demo-up DEMO_MODE=agent_real_langchain_ts` green; SQL row + ordering check both green.
- [ ] `make demo-up DEMO_MODE=agent_real_langchain_ts SPENDGUARD_DEMO_DENY=1` proves the denied-budget short-circuit (0 OpenAI requests).
- [ ] `README.md` (repo root) `## Adapter integrations` table has the TS row.
- [ ] `docs/site/docs/integrations/langchain.md` has a TS section.

When the checklist is fully green D04 is **shipped** per build-plan §7 definition of done, and the `project_coverage_D04_shipped.md` memory entry is written per build-plan §8.
