# COV_S05_01 — D05 TS SDK substrate: package init

> **Deliverable**: D05 TS SDK substrate
> **Slice**: 1 of 10
> **Size**: S
> **Spec set**: [`docs/specs/coverage/D05_ts_sdk_substrate/`](../specs/coverage/D05_ts_sdk_substrate/)

## Scope

Lay down the bare TypeScript package skeleton for `@spendguard/sdk` so subsequent slices have a place to write code.

Concretely:
- Create `sdk/typescript/` workspace carve-out (no Cargo workspace conflict).
- Write `package.json` with:
  - `name`: `@spendguard/sdk`
  - `version`: `0.0.0` (real `0.1.0` ships in S05_10)
  - `type`: `"module"` (ESM-only per locked decision §9.1)
  - `engines.node`: `>=20.10.0`
  - empty `exports` map (subsequent slices populate)
  - `peerDependencies`: `@grpc/grpc-js >=1.10`, `@grpc/proto-loader >=0.7`
  - `peerDependenciesMeta`: OTel marked optional (locked decision §9.7)
  - `scripts`: `build`, `test`, `lint`, `typecheck`, `size`, all placeholder NO-OPs that exit 0
- `tsconfig.json`: strict mode, target `ES2022`, module `NodeNext`, no decorators.
- `tsup.config.ts`: ESM-only build, target Node 20, source maps on.
- `biome.json`: locked formatter + linter rules (Biome over ESLint+Prettier per §9.5).
- `vitest.config.ts`: ESM, Node env, isolated per-file workers.
- `README.md`: placeholder pointing at design doc.
- `pnpm-workspace.yaml` at repo root: add `sdk/typescript/` to the workspace.

## Files touched

| File | Why |
|------|-----|
| `sdk/typescript/package.json` | Manifest |
| `sdk/typescript/tsconfig.json` | TS compiler config |
| `sdk/typescript/tsup.config.ts` | Build config |
| `sdk/typescript/biome.json` | Lint/format |
| `sdk/typescript/vitest.config.ts` | Test runner config |
| `sdk/typescript/README.md` | Placeholder |
| `pnpm-workspace.yaml` (NEW or UPDATE) | Workspace registration |

## Test/verification plan

This slice produces no runtime code, so test gates are limited to:

1. `cd sdk/typescript && pnpm install` succeeds without unmet peer-dep warnings.
2. `cd sdk/typescript && pnpm run build` exits 0 (placeholder build).
3. `cd sdk/typescript && pnpm run test` exits 0 (no tests yet; vitest reports 0 passed).
4. `cd sdk/typescript && pnpm run lint` exits 0.
5. `cd sdk/typescript && pnpm run typecheck` exits 0 against an empty `src/`.
6. `git ls-files sdk/typescript/` shows ONLY the 6 files above + `pnpm-workspace.yaml` change.
7. `pnpm install` from repo root recognizes the new workspace member.

## Anti-scope

Explicitly NOT in this slice:

- Any `src/*.ts` files — those come in S05_02 through S05_08.
- `tests/` content — comes in S05_09.
- npm publish workflow — comes in S05_10.
- Proto codegen pipeline — comes in S05_02.
- `pnpm-lock.yaml` regeneration if it exists is incidental; intentional lock update is fine.

## Backlinks

- Spec set: [`design.md`](../specs/coverage/D05_ts_sdk_substrate/design.md) §6.1 toolchain decisions, §8 slice plan, §9 locked design decisions
- Build plan: [`framework-coverage-build-plan-2026-06.md`](../strategy/framework-coverage-build-plan-2026-06.md) §1.5 slice doc convention
- Review standards: [`review-standards.md`](../specs/coverage/D05_ts_sdk_substrate/review-standards.md) — apply universal repo standards + slice-specific package-init checks
