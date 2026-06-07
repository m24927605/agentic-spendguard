// tsup build configuration for @spendguard/openai-agents.
//
// SLICE 1 + 2 ships the package skeleton + `withSpendGuard` factory +
// `SpendGuardAgentsModel` class + `runContext()` AsyncLocalStorage shim.
// Entry map: barrel + the `./run-context` subpath alias the design.md §6
// locks for cross-package run-context sharing with D04 / D06 / D29.
//
// Locked decisions (mirrors @spendguard/vercel-ai D06 + @spendguard/langchain
// D04 + @spendguard/sdk D05 substrates):
//   - ESM-only output (review-standards.md §4.5).
//   - Node 20 target floor (peer-aligned with @spendguard/sdk).
//   - `splitting: true` so the `index.ts` and `runContext.ts` entries SHARE
//     the underlying chunk at runtime instead of each entry getting its own
//     inlined copy of the `AsyncLocalStorage` storage helper. Without
//     splitting, the `index.ts` barrel ships its own `storage()` closure
//     and the `/run-context` subpath ships ANOTHER, so a multi-framework
//     run that imports `runContext` from both entries fails the
//     `Symbol.for("@spendguard/run-context/v1")` global-registry parity
//     check (design.md §7 locked decision #4). Splitting lifts the shared
//     symbol into a chunk that both entries import.
//   - `@openai/agents` + `@spendguard/sdk` are `external` — peer deps,
//     never bundled (review-standards.md §4.7 / §5.1-5.4).

import { defineConfig } from "tsup";

export default defineConfig({
  entry: {
    index: "src/index.ts",
    runContext: "src/runContext.ts",
  },
  format: ["esm"],
  dts: true,
  clean: true,
  sourcemap: false,
  splitting: true,
  target: "node20",
  treeshake: true,
  external: ["@openai/agents", "@spendguard/sdk"],
});
