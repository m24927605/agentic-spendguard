# COV_D38_01 — D38 Mastra adapter: package init

> **Deliverable**: D38 Mastra dedicated adapter (`@spendguard/mastra`)
> **Slice**: 1 of 7 (S — skeleton + config, no behavior)
> **Spec set**: [`docs/specs/coverage/D38_mastra/`](../../specs/coverage/D38_mastra/)
> **Precedence**: `design.md` is LOCKED and trumps this doc (review-standards §1). Any disagreement here is a slice-author bug — follow design.md and flag the drift.

## Scope

Initialize the `sdk/typescript-mastra/` package skeleton as a new pnpm workspace member: `package.json` (per the implementation.md §2 skeleton), tsconfig pair, tsup/biome/vitest configs, `scripts/` (size-budget, version-check, prepublish — copied from `sdk/typescript-langchain/scripts`), placeholder barrel (`src/index.ts`), `src/version.ts` (`VERSION` constant), `src/errors.ts` (three-class substrate re-export), and a sanity import test. After this slice the package installs, lints, typechecks (including against the installed `@mastra/core` `^1.41.0` devDep), and builds — with zero processor logic.

The placeholder barrel is a strict SUBSET of the design §5 LOCKED barrel: `VERSION` + the three error re-exports. The `SpendGuardProcessor` / options exports land in COV_D38_02, completing the §5 verbatim shape. The barrel never contains anything NOT in §5.

## Files touched

Exact set per implementation.md §1 / §8 (slice row COV_D38_01):

| File | Why |
|------|-----|
| `sdk/typescript-mastra/package.json` | NEW — implementation.md §2 skeleton verbatim (name `@spendguard/mastra`, version `0.1.0`, Apache-2.0, ESM-only, engines `>=22.13.0`, peers `@mastra/core >=1.0.0 <2` + `@spendguard/sdk workspace:*`) |
| `sdk/typescript-mastra/tsconfig.json` + `tsconfig.tests.json` | NEW — strict + nodenext ESM, D04/D06 discipline |
| `sdk/typescript-mastra/tsup.config.ts` | NEW — ESM-only, `external: ["@mastra/core", "@spendguard/sdk"]`, no CJS artifact (implementation.md §7) |
| `sdk/typescript-mastra/biome.json` | NEW — mirror sibling adapter lint config |
| `sdk/typescript-mastra/vitest.config.ts` | NEW — coverage thresholds wired to tests.md §1 floors |
| `sdk/typescript-mastra/scripts/{size-budget.sh,version-check.sh,prepublish.sh}` | NEW — copied from `sdk/typescript-langchain/scripts` |
| `sdk/typescript-mastra/src/index.ts` | NEW — placeholder barrel (subset of design §5; completed in COV_D38_02) |
| `sdk/typescript-mastra/src/version.ts` | NEW — `VERSION` constant (version-check.sh keeps in sync) |
| `sdk/typescript-mastra/src/errors.ts` | NEW — implementation.md §3.6 re-export |
| `sdk/typescript-mastra/tests/` sanity import test | NEW — pre-TP-01 smoke (barrel imports, `VERSION` exported) |
| `sdk/typescript-mastra/README.md` | NEW — placeholder (full content lands in COV_D38_06) |
| `pnpm-workspace.yaml` | + `sdk/typescript-mastra` member |

## LOCKED surface quoted verbatim

### Package facts — design.md §3 goal 1

> Publish `@spendguard/mastra` npm package, version `0.1.0`, Apache-2.0, in-tree at `sdk/typescript-mastra/` (new pnpm workspace member). Peer-deps: `@mastra/core` `>=1.0.0 <2`, `@spendguard/sdk` (workspace convention identical to D06's published shape). Node `>=22.13.0` (Mastra 1.x floor).

And design.md §11.14:

> **Node engine `>=22.13.0`**, peer `@mastra/core >=1.0.0 <2`, ESM-only, tsup/vitest/biome — D04/D06 package discipline otherwise.

Implementation.md §2 note (review gate): "Node floor `>=22.13.0` is the Mastra 1.x requirement, NOT the D04/D06 `>=20.10` — review gate asserts nobody \"harmonizes\" it downward."

### `package.json` skeleton — implementation.md §2 (copy verbatim)

```json
{
  "name": "@spendguard/mastra",
  "version": "0.1.0",
  "description": "Mastra Processor for SpendGuard budget guardrails — hard, fail-closed, pre-dispatch budget reservation for Mastra Agents (model-router strings included)",
  "license": "Apache-2.0",
  "author": "Michael Chen <m24927605@gmail.com>",
  "homepage": "https://github.com/m24927605/agentic-spendguard",
  "repository": {
    "type": "git",
    "url": "https://github.com/m24927605/agentic-spendguard.git",
    "directory": "sdk/typescript-mastra"
  },
  "bugs": "https://github.com/m24927605/agentic-spendguard/issues",
  "keywords": ["llm", "agent", "spend", "budget", "spendguard", "mastra", "processor", "guardrails"],
  "type": "module",
  "engines": { "node": ">=22.13.0" },
  "sideEffects": false,
  "publishConfig": { "access": "public", "provenance": true },
  "files": ["dist/**/*.js", "dist/**/*.d.ts", "README.md", "LICENSE_NOTICES.md", "CHANGELOG.md"],
  "main": "./dist/index.js",
  "module": "./dist/index.js",
  "types": "./dist/index.d.ts",
  "exports": {
    ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" },
    "./package.json": "./package.json"
  },
  "scripts": {
    "build": "tsup",
    "test": "vitest run",
    "test:watch": "vitest",
    "lint": "biome check src tests",
    "format": "biome format --write src tests",
    "typecheck": "tsc --noEmit && tsc -p tsconfig.tests.json --noEmit",
    "size": "bash scripts/size-budget.sh",
    "version-check": "bash scripts/version-check.sh",
    "prepublishOnly": "bash scripts/prepublish.sh"
  },
  "peerDependencies": {
    "@mastra/core": ">=1.0.0 <2",
    "@spendguard/sdk": "workspace:*"
  },
  "devDependencies": {
    "@biomejs/biome": "^1.9.4",
    "@mastra/core": "^1.41.0",
    "@spendguard/sdk": "workspace:*",
    "@types/node": "^22.10.0",
    "tsup": "^8.3.0",
    "typescript": "^5.6.0",
    "vitest": "^2.1.0"
  }
}
```

### `src/errors.ts` — implementation.md §3.6 (copy verbatim)

```ts
export { DecisionDenied, SidecarUnavailable, SpendGuardError } from "@spendguard/sdk";
```

> Direct re-export (class identity preserved; `instanceof` works across the boundary). Exactly the D06 three-class anti-list; everything else imports from the substrate.

### Bundle budget — implementation.md §2

> **Bundle budget: 40 KB minified, 12 KB gzipped** for `dist/index.js` (D04 parity — thin glue; `@mastra/core` and `@spendguard/sdk` are externalized peers). Budget breach fails the build via `size-budget.sh` wired into `prepublishOnly`.

(The size gate A2.5 is *executed* as a ship gate in COV_D38_06; this slice ships the script and the `prepublishOnly` wiring.)

## VERIFY-AT-IMPL pins owned by this slice

None. No design §12 marker is pinned by COV_D38_01.

## Test/verification plan (tests.md §4)

| ID | One-liner |
|----|-----------|
| sanity import test (pre-TP-01) | barrel imports cleanly under vitest; `VERSION` exported; error re-exports reference-identical to `@spendguard/sdk` |

## Acceptance gates (acceptance.md §8 subset: A2.1..A2.4, A2.6, A2.7; sanity import test)

```sh
# A2.1 — workspace member resolves
pnpm install --frozen-lockfile          # repo root; pnpm-workspace.yaml includes sdk/typescript-mastra

# A2.2 / A2.3 / A2.4 — lint, typecheck, build
pnpm -C sdk/typescript-mastra run lint
pnpm -C sdk/typescript-mastra run typecheck
pnpm -C sdk/typescript-mastra run build   # dist/index.js + dist/index.d.ts, ESM-only, no CJS

# A2.6 / A2.7 — engines + peer range
node -p 'require("./sdk/typescript-mastra/package.json").engines.node'                       # >=22.13.0
node -p 'require("./sdk/typescript-mastra/package.json").peerDependencies["@mastra/core"]'   # >=1.0.0 <2

# sanity import test
pnpm -C sdk/typescript-mastra run test
```

## Anti-scope (review-standards §13 row COV_D38_01)

- Skeleton ONLY — no `SpendGuardProcessor`, no options type, no identity/flatten/inflight modules (COV_D38_02).
- No commit-path / usage code (COV_D38_03); no fail-closed matrix or hash-reuse tests (COV_D38_04).
- No demo overlay / example runner / Makefile changes (COV_D38_05).
- No README content, docs page, publish workflow, or repo-root adapter row (COV_D38_06).
- No per-chunk stream gating, auxiliary-LLM coverage, or AI SDK v6 V3 middleware — out of D38 entirely (design §4, §9.3).
- `deploy/demo/vercel_ai_mastra/**` + `verify_step_vercel_ai_mastra.sql` byte-untouched (design §9.4).
- Do NOT "harmonize" the Node floor down to 20.10 (TA-11 guard).

## Backlinks

- [`design.md`](../../specs/coverage/D38_mastra/design.md) — §3.1, §11.1, §11.14, §13 (slice plan)
- [`implementation.md`](../../specs/coverage/D38_mastra/implementation.md) — §1 (layout), §2 (package.json skeleton), §3.6 (errors.ts), §7 (bundle hygiene), §8 (slice → file map)
- [`tests.md`](../../specs/coverage/D38_mastra/tests.md) — §4 (sanity import test row)
- [`acceptance.md`](../../specs/coverage/D38_mastra/acceptance.md) — §2 (A2.1..A2.7), §8 (slice subsets)
- [`review-standards.md`](../../specs/coverage/D38_mastra/review-standards.md) — §10 (package hygiene), §13 (anti-scope)
