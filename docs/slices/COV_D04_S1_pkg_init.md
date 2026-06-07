# COV_D04_S1 — D04 LangChain TS: package init

> **Deliverable**: D04 LangChain TS adapter
> **Slice**: 1 of 6 (S)
> **Spec set**: [`docs/specs/coverage/D04_langchain_ts/`](../specs/coverage/D04_langchain_ts/)

## Scope

Initialize `@spendguard/langchain` npm package skeleton. Mirrors D05 package patterns (tsup ESM, biome lint, vitest, @spendguard/sdk peer dep). After SLICE 1, the package builds, lints, tests (empty), and is publishable as 0.1.0-pre.

Concretely:
- `sdk/typescript-langchain/package.json` — NEW (name `@spendguard/langchain`, version `0.1.0-pre`, peer deps: `@spendguard/sdk` workspace + `@langchain/core` >= 0.3, `@langchain/openai` optional dev-dep for tests)
- `sdk/typescript-langchain/tsconfig.json` + `tsconfig.tests.json` — mirror D05 (strict + nodenext + ESM)
- `sdk/typescript-langchain/tsup.config.ts` — entry: src/index.ts; ESM + DTS
- `sdk/typescript-langchain/biome.json` — mirror D05 lint config
- `sdk/typescript-langchain/vitest.config.ts` — mirror D05
- `sdk/typescript-langchain/src/index.ts` — empty barrel (`export {}` placeholder)
- `sdk/typescript-langchain/src/version.ts` — `VERSION = "0.1.0-pre"`
- `sdk/typescript-langchain/tests/locked-surface.test.ts` — assert barrel exists + version exported
- `sdk/typescript-langchain/README.md` — placeholder (full content lands in SLICE 6)
- (Workspace) `pnpm-workspace.yaml` — add `sdk/typescript-langchain` package

## Files touched

| File | Why |
|------|-----|
| `sdk/typescript-langchain/package.json` | NEW package |
| `sdk/typescript-langchain/tsconfig{,.tests}.json` | NEW |
| `sdk/typescript-langchain/tsup.config.ts` | NEW build |
| `sdk/typescript-langchain/biome.json` | NEW lint |
| `sdk/typescript-langchain/vitest.config.ts` | NEW test |
| `sdk/typescript-langchain/src/{index,version}.ts` | NEW barrel + version |
| `sdk/typescript-langchain/tests/locked-surface.test.ts` | smoke |
| `sdk/typescript-langchain/README.md` | placeholder |
| `pnpm-workspace.yaml` | workspace registration |

## Test/verification plan

1. `cd sdk/typescript-langchain && pnpm run typecheck` clean
2. `pnpm run lint` clean
3. `pnpm run build` clean
4. `pnpm run test` — locked-surface smoke passes
5. `pnpm pack --dry-run` shows expected files

## Anti-scope

- No SpendGuardCallbackHandler body — SLICE 2
- No reserve/commit wiring — SLICE 3
- No docs page — SLICE 6
- No publish workflow — SLICE 6

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D04_langchain_ts/design.md) §6 slice plan, §5 architecture
- Peer dep target: `@spendguard/sdk@0.1.0` (D05 just shipped 2026-06-07)
