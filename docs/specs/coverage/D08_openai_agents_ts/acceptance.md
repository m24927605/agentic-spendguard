# D08 — Acceptance Gates

These are the gates `superpowers:code-reviewer` re-runs to confirm D08 is shipped. Every gate is runnable in the current repo state at slice-spec time per build plan §3. No gate depends on third-party action SpendGuard cannot trigger (e.g. "OpenAI merges X" is not a gate).

## 1. Build + lint + typecheck

| Gate | Command (run from `sdk/typescript/packages/openai-agents/`) | Pass condition |
|---|---|---|
| A1.1 | `pnpm install --frozen-lockfile` | exit 0; lockfile drift = fail |
| A1.2 | `pnpm run lint` | biome zero diagnostics |
| A1.3 | `pnpm run typecheck` | `tsc --noEmit` exit 0 |
| A1.4 | `pnpm run build` | tsup produces `dist/index.js`, `dist/model.js`, `dist/runContext.js`, plus each `.d.ts` |
| A1.5 | `pnpm run size` | minified ≤ 60 KB, gzipped ≤ 18 KB |
| A1.6 | `pnpm run typecheck` with `@openai/agents@0.3.0` resolved | passes |
| A1.7 | `pnpm run typecheck` with simulated `@openai/agents@0.4.x` declarations | passes — peer-range honoured |

## 2. Test gates

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | `pnpm run test` | vitest exit 0; ≥ 90 % statements, ≥ 85 % branches |
| A2.2 | `pnpm run test tests/withSpendGuard.test.ts tests/model.test.ts` | WS-01..WS-15 + M-01..M-15 all pass |
| A2.3 | `pnpm run test tests/runContext.test.ts` | RC-01..RC-08 pass — including shared-key RC-07 |
| A2.4 | `pnpm run test tests/crossLanguageSignature.test.ts` | ≥ 32 vectors pass — byte-for-byte parity with Python |
| A2.5 | `pnpm run test tests/defaultEstimator.test.ts` | DE-05 (Python parity table read) passes |

## 3. Behaviour invariant (P0)

The load-bearing claim of the adapter:

> **SpendGuard DENY ⇒ inner `Model` is NEVER invoked.**

| Gate | Command | Pass condition |
|---|---|---|
| A3.1 | `pnpm run test tests/withSpendGuard.test.ts -t "WS-02"` | DENY → `mockInnerModel.callCount === 0` |
| A3.2 | `pnpm run test tests/withSpendGuard.test.ts -t "WS-03"` | STOP → `mockInnerModel.callCount === 0` |
| A3.3 | `pnpm run test tests/withSpendGuard.test.ts -t "WS-04"` | SKIP → `mockInnerModel.callCount === 0` |
| A3.4 | `pnpm run test tests/model.test.ts -t "M-02"` | DENY in subclass form → `mockInnerModel.callCount === 0` |
| A3.5 | `make demo-ts-composite-mock` | demo `--mock` mode asserts the same invariant end-to-end |

## 4. Cross-language determinism (P0)

| Gate | Path / Command | Pass condition |
|---|---|---|
| A4.1 | `sdk/fixtures/cross-language/v1.json` `openai_agents` section | ≥ 32 vectors covering string + object inputs |
| A4.2 | `pnpm run test tests/crossLanguageSignature.test.ts` | all vectors green |
| A4.3 | `make -C sdk/python test PYTHONPATH=src TEST=tests/integrations/test_openai_agents_cross_language.py` | same fixture, same outputs, all green |
| A4.4 | Manual: pick three random vectors; recompute by hand using Python blake2b + uuid derivation. Compare TS output. | byte-identical |
| A4.5 | `defaultEstimator.MODEL_BASELINE_TOKENS` TS dict vs Python dict (DE-05 test) | identical key/value pairs |

## 5. Demo end-to-end

| Gate | Command | Pass condition |
|---|---|---|
| A5.1 | `make demo-ts-build` | `sdk/typescript` + `examples/openai-agents-ts-composite` both build clean |
| A5.2 | `make demo-ts-composite-mock` | demo `--mock` exits 0 with "All paths PASS" |
| A5.3 | `DEMO_MODE=agent_real_openai_agents_ts OPENAI_API_KEY=$OPENAI_API_KEY_TEST make demo-up` | exits 0 with "Runner.run OK" line; demo Postgres `audit_outbox` row count for the run matches PRE+POST pattern |
| A5.4 | `psql -c "SELECT trigger, decision FROM audit_outbox WHERE run_id = '<demo run>' ORDER BY emitted_at"` | exactly 2 rows: `LLM_CALL_PRE` (CONTINUE) + `LLM_CALL_POST` |
| A5.5 | `services/sidecar/scripts/verify-chain.sh --run-id <demo run>` | audit-chain HMAC verifies for the new rows |

A5.3-A5.5 require `OPENAI_API_KEY_TEST` in the demo CI shard env (mirrors existing `agent_real_openai_agents` mode). The mode is skipped when missing.

## 6. Public-surface contract

| Gate | Command | Pass condition |
|---|---|---|
| A6.1 | `node -e 'import("@spendguard/openai-agents").then(m => console.log(Object.keys(m).sort()))'` (after `pnpm pack && pnpm add ./spendguard-openai-agents-0.1.0.tgz` in tmp) | output includes `SpendGuardAgentsModel`, `currentRunContext`, `runContext`, `withSpendGuard` |
| A6.2 | `node -e 'import("@spendguard/openai-agents/model").then(m => console.log(Object.keys(m).sort()))'` | output is `["SpendGuardAgentsModel"]` |
| A6.3 | `node -e 'import("@spendguard/openai-agents/run-context").then(m => console.log(Object.keys(m).sort()))'` | output is `["currentRunContext","runContext"]` |
| A6.4 | `tests/_support/contractSnapshot.ts` (committed in S08_03) | typechecks at the declared v0.1.0 surface |

## 7. Publish-pipeline dry-run

| Gate | Command | Pass condition |
|---|---|---|
| A7.1 | `pnpm pack` | produces `spendguard-openai-agents-0.1.0.tgz`; tarball includes `dist/`, `README.md`, `LICENSE_NOTICES.md`, `CHANGELOG.md` only |
| A7.2 | `tar -tzf spendguard-openai-agents-0.1.0.tgz \| grep -E "(src/\|tests/\|node_modules)"` | empty output |
| A7.3 | `du -k spendguard-openai-agents-0.1.0.tgz` | ≤ 200 KB |
| A7.4 | `cat .github/workflows/sdk-ts-openai-agents-publish.yml` | `id-token: write`, `npm publish --provenance`, release-tag prefix `ts-sdk-openai-agents-v*` |
| A7.5 | `gh workflow run sdk-ts-openai-agents-publish.yml --ref <branch>` (workflow_dispatch) | reaches `Publish (provenance)` step, CI green for prior steps |
| A7.6 | `tar -tzf <tarball> \| grep -E "(@openai/agents\|@spendguard/sdk)"` | empty — peer deps NOT bundled |

## 8. Documentation gates

| Gate | Path | Pass condition |
|---|---|---|
| A8.1 | `sdk/typescript/packages/openai-agents/README.md` | install command + ≤ 40-line quickstart using `withSpendGuard` |
| A8.2 | `sdk/typescript/packages/openai-agents/CHANGELOG.md` | 0.1.0 entry: "first public release; mirrors Python `spendguard.integrations.openai_agents`" |
| A8.3 | `sdk/typescript/packages/openai-agents/LICENSE_NOTICES.md` | notices for `@noble/hashes`; peer deps `@openai/agents` + `@spendguard/sdk` listed under "Peer dependencies" header |
| A8.4 | `docs/site/docs/integrations/openai-agents-ts.md` | published doc with install / quickstart / behaviour contract / demo command |
| A8.5 | `README.md` (repo root) `## 🔌 Adapter integrations` table | row for `@spendguard/openai-agents` linking to npm + integration doc |
| A8.6 | `examples/openai-agents-ts-composite/README.md` | parallels Python composite README — what-it-proves / two-modes / expected output |

## 9. Slice-level acceptance subset

| Slice | Subset acceptance |
|---|---|
| `COV_S08_01_d08_package_init` | A1.1, A1.2, A1.3 pass; `package.json` matches `implementation.md` §2 |
| `COV_S08_02_d08_with_spendguard_factory` | `pnpm run build` clean; smoke test that `withSpendGuard(mockInner, mockOpts).getResponse(...)` runs |
| `COV_S08_03_d08_tests_unit` | A2.1, A2.2, A2.3, A2.4, A2.5, A3.1-A3.4 pass; fixture extended |
| `COV_S08_04_d08_demo_mock_real` | A5.1, A5.2 pass; mock demo asserts invariant |
| `COV_S08_05_d08_demo_mode_wireup` | A5.3, A5.4, A5.5 pass (skipped only when `OPENAI_API_KEY_TEST` unset, mirroring `agent_real_openai_agents`) |
| `COV_S08_06_d08_docs_publish` | A1.4, A1.5, A7.1-A7.6, A8.1-A8.6 pass |

## 10. Ship-readiness summary checklist

The reviewer signs off only when all of the following are true:

- [ ] Every gate in §1–§8 is green.
- [ ] `git status` shows no uncommitted files under `sdk/typescript/packages/openai-agents/` or `examples/openai-agents-ts-composite/`.
- [ ] `git log --oneline sdk/typescript/packages/openai-agents/` shows ≥ 6 atomic commits (one per slice).
- [ ] A `ts-sdk-openai-agents-v0.1.0` git tag exists on the slice-S08_06 merge commit.
- [ ] The publish workflow has been dry-run via `workflow_dispatch` on at least one PR; the run reached the npm-publish step.
- [ ] `tests/crossLanguageSignature.test.ts` consumes the same `sdk/fixtures/cross-language/v1.json` the Python suite consumes; no fixture drift.
- [ ] `README.md` (repo root) adapter table has a row for `@spendguard/openai-agents`.
- [ ] The new `DEMO_MODE=agent_real_openai_agents_ts` mode passes in CI (or is documented as skipped because `OPENAI_API_KEY_TEST` is unset in that shard).
- [ ] The Python sibling memory entry `feedback_demo_quality_gate` invariant holds: codex green is not enough — the demo actually runs and the audit-chain verifies.

When this checklist is fully green the adapter is **shipped** per build-plan §7 definition of done.
