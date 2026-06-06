# D05 — Acceptance Gates

These are the gates a reviewer (`superpowers:code-reviewer`) re-runs to confirm D05 is shipped. Every gate must be runnable in the current repo state at slice-spec time per build plan §3 (`framework-coverage-build-plan-2026-06.md`). No gate depends on third-party action SpendGuard cannot trigger.

## 1. Build + lint + typecheck gates

| Gate | Command (run from `sdk/typescript/`) | Pass condition |
|---|---|---|
| A1.1 | `pnpm install --frozen-lockfile` | exit 0; lockfile drift = fail |
| A1.2 | `pnpm run proto && git diff --exit-code src/_proto` | exit 0; committed codegen matches proto sources |
| A1.3 | `pnpm run lint` | biome zero diagnostics |
| A1.4 | `pnpm run typecheck` | `tsc --noEmit` exit 0 |
| A1.5 | `pnpm run build` | tsup produces `dist/index.js`, `dist/index.d.ts`, plus each subpath in `package.json#exports`, exit 0 |
| A1.6 | `pnpm run size` | `dist/index.js` minified ≤ 120 KB; gzipped ≤ 35 KB |

## 2. Test gates

| Gate | Command | Pass condition |
|---|---|---|
| A2.1 | `pnpm run test` | vitest exit 0; ≥ 85 % statements, ≥ 80 % branches, ≥ 85 % functions, ≥ 85 % lines coverage |
| A2.2 | `pnpm run test tests/crossLanguage.test.ts` | All ≥ 64 cross-language vectors pass — byte-for-byte parity with Python + Rust |
| A2.3 | `pnpm run test tests/treeShaking.test.ts` | Subpath imports do not pull `@grpc/grpc-js` for non-client modules; full surface ≤ 120 KB / 35 KB gz |
| A2.4 | `pnpm run test:e2e` | `e2e/reserveCommitRelease.test.ts` runs the full reserve → commit → release cycle against the mock sidecar; ack counter > 0; clean shutdown |

## 3. Determinism / parity gates (P0)

These three gates are the cross-language audit-chain invariants. They MUST hold or the substrate is broken regardless of test count.

| Gate | Path | Pass condition |
|---|---|---|
| A3.1 | `sdk/fixtures/cross-language/v1.json` | File exists, committed; ≥ 64 fixture entries split across `computePromptHash`, `deriveIdempotencyKey`, `defaultCallSignature` |
| A3.2 | `pnpm run test tests/crossLanguage.test.ts` + `make -C sdk/python test PYTHONPATH=src TEST=tests/cross_language_test.py` + `cargo test --package sidecar cross_language -- --include-ignored` | All three suites parse the same fixture file and produce zero diffs |
| A3.3 | Manual: pick three random fixture entries; recompute by hand using Python `hashlib`/`hmac`. Compare with TS output. | Match. |

A3.2 specifically: the Python + Rust suites already exist before D05. Slice S05_09 adds the TS suite. The fixture itself is created in slice S05_09 if not yet present; otherwise reused.

## 4. Public surface contract gates

The four downstream deliverables (D04 / D06 / D08 / D29) build against the symbols in `design.md` §4. The contract is met iff every documented symbol is exported at the documented type with the documented semantic.

| Gate | Command | Pass condition |
|---|---|---|
| A4.1 | `pnpm run typecheck` over `tests/_support/contractSnapshot.ts` | A snapshot file in the test tree imports every public symbol with its documented signature; typecheck passes |
| A4.2 | `pnpm pack && tar -tzf spendguard-sdk-0.1.0.tgz \| grep -E "(client\|errors\|ids\|pricing\|promptHash\|runPlan)\.(js\|d\.ts)" \| wc -l` | ≥ 12 (each subpath has both `.js` and `.d.ts`) |
| A4.3 | `node -e 'import("@spendguard/sdk").then(m => console.log(Object.keys(m).sort()))'` (after `pnpm pack && pnpm add ./spendguard-sdk-0.1.0.tgz` in a tmp dir) | Output includes every symbol enumerated in `design.md` §4.1 |
| A4.4 | `pnpm run typecheck` with a sample adapter shim file (committed under `tests/_support/sampleAdapter.ts`) that imports + uses `SpendGuardClient.reserve / commit / release / queryBudget` against the v0.1.0 types | Typecheck passes — this is the D04/D06/D08/D29-readiness gate |

## 5. Publish-pipeline dry-run

| Gate | Command | Pass condition |
|---|---|---|
| A5.1 | `pnpm pack` | Produces `spendguard-sdk-0.1.0.tgz`; tarball includes `dist/`, `README.md`, `LICENSE_NOTICES.md`, `CHANGELOG.md` and NOTHING under `src/`, `tests/`, `node_modules/`, `scripts/`, `.github/` |
| A5.2 | `tar -tzf spendguard-sdk-0.1.0.tgz \| grep -E "(src/\|tests/\|node_modules)"` | empty output |
| A5.3 | `du -k spendguard-sdk-0.1.0.tgz` | ≤ 800 KB (tarball; well under the 5 MB installed budget) |
| A5.4 | `cat .github/workflows/sdk-ts-publish.yml` | OIDC `id-token: write` permission; `npm publish --provenance`; release-trigger gated on `ts-sdk-v*` tag prefix |
| A5.5 | `gh workflow run sdk-ts-publish.yml --ref <branch>` triggered manually (workflow_dispatch) | Reaches the `Publish (provenance)` step (npm publish skipped in test branch if env not set), CI green for everything before it |

## 6. Runtime matrix gates

| Gate | Command | Pass condition |
|---|---|---|
| A6.1 | Node 20.10 CI shard `pnpm run test` | exit 0 |
| A6.2 | Node 22 LTS CI shard `pnpm run test` and `pnpm run test:e2e` | exit 0 |
| A6.3 | Bun 1.1+ CI shard `bun test tests/{ids,errors,pricing,promptHash,runPlan,crossLanguage}.test.ts` | exit 0 (subset only — advisory in v0.1.0) |
| A6.4 | Deno 1.46+ CI shard `deno test --allow-env --allow-read tests/{ids,errors,pricing,promptHash,runPlan,crossLanguage}.test.ts` | exit 0 (subset only — advisory in v0.1.0) |

A6.3 / A6.4 are advisory (not blocking) in v0.1.0; promoted to blocking in v0.2.

## 7. Documentation gates

| Gate | Path | Pass condition |
|---|---|---|
| A7.1 | `sdk/typescript/README.md` | Includes install command, 30-line quickstart that demonstrates `reserve → commitEstimated → release` |
| A7.2 | `sdk/typescript/CHANGELOG.md` | 0.1.0 entry present with "first public release; mirrors `spendguard-sdk` (Python)" preamble |
| A7.3 | `sdk/typescript/LICENSE_NOTICES.md` | Lists `@grpc/grpc-js`, `@protobuf-ts/runtime`, `@noble/hashes` notices |
| A7.4 | `README.md` (repo root) — `## Adapter integrations` table | Includes a row for `@spendguard/sdk` (TS) pointing to the npm page |
| A7.5 | `docs/site/docs/integrations/typescript-sdk.md` | Page exists with install / config / quickstart; matches the build-plan §7 definition-of-done item |

## 8. Backlog-coverage gate

D05 unblocks D04, D06, D08, D29. The gate verifies the substrate is sufficient for each:

| Gate | Command | Pass condition |
|---|---|---|
| A8.1 | `tests/_support/sampleAdapter.ts` imports `SpendGuardClient` and writes a callback handler in the shape `D04` requires | TS compiles, mock-sidecar e2e passes |
| A8.2 | Same file writes a `middleware` wrapper in the shape `D06` (Vercel AI SDK) requires | TS compiles |
| A8.3 | Same file writes a `Model` subclass in the shape `D08` (OpenAI Agents TS) requires | TS compiles |
| A8.4 | Same file writes a `step.ai.wrap` adapter in the shape `D29` (Inngest AgentKit) requires | TS compiles |

These are **type-level** gates only — the four sample adapters are not real adapter packages, they are sketches to prove the substrate's surface is sufficient. The full implementations live in D04/D06/D08/D29.

## 9. Slice-level acceptance subset

Per build-plan §1.4 the reviewer also asserts each slice's own scope:

| Slice | Subset acceptance |
|---|---|
| `COV_S05_01_d05_package_init` | A1.1, A1.3, A1.4 pass. |
| `COV_S05_02_d05_proto_codegen` | A1.2 passes. Generated tree under `src/_proto/` exists and compiles. |
| `COV_S05_03_d05_client_skeleton` | C-01..C-05, C-31..C-34, EN-01..EN-05 pass. |
| `COV_S05_04_d05_handshake_reserve_commit` | C-05..C-23 pass. |
| `COV_S05_05_d05_release_query` | C-19..C-21, C-26..C-30 pass. |
| `COV_S05_06_d05_ids_prompt_hash_pricing` | I-01..I-11, P-01..P-07, PR-01..PR-06 pass. |
| `COV_S05_07_d05_run_plan` | R-01..R-08 pass. |
| `COV_S05_08_d05_otel_retry_idempotency` | O-01..O-06, RT-01..RT-08, DC-01..DC-05 pass. |
| `COV_S05_09_d05_test_matrix` | A2.1, A2.2, A3.1..A3.3, fixture file committed, all cross-language gates green. |
| `COV_S05_10_d05_publish_pipeline` | A1.5, A1.6, A5.1..A5.5, A7.1..A7.5 pass. |

## 10. Ship-readiness summary checklist

The reviewer signs off only when all of the following are true:

- [ ] Every gate in §1–§8 is green.
- [ ] `git status` shows no uncommitted files under `sdk/typescript/`.
- [ ] `git log --oneline sdk/typescript/` shows ≥ 10 atomic commits (one per slice).
- [ ] A `ts-sdk-v0.1.0` git tag exists on the slice-S05_10 merge commit.
- [ ] The publish workflow has been dry-run via `workflow_dispatch` on at least one PR; the run reached the npm-publish step (skipped when not on a real release).
- [ ] The TS substrate's `crossLanguage.test.ts` consumes the same `sdk/fixtures/cross-language/v1.json` file the Python + Rust suites consume; no fixture drift.
- [ ] `tests/_support/sampleAdapter.ts` is committed and proves D04/D06/D08/D29-readiness at the type level.
- [ ] `tests/_support/contractSnapshot.ts` is committed and exhaustively covers every symbol from `design.md` §4.
- [ ] `README.md` (repo root) `## Adapter integrations` table has a row pointing to `@spendguard/sdk` on npm.

When the checklist is fully green the substrate is **shipped** per build-plan §7 definition of done, and D04 / D06 / D08 / D29 specs may proceed.
