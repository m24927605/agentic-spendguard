// tsup build configuration for @spendguard/vercel-ai.
//
// SLICE 1 shipped the package skeleton — entry map was just the barrel.
// SLICE 7 adds the `./mastra` subpath alias entry (the Mastra-side
// function-reference re-export of `createSpendGuardMiddleware`).
//
// Locked decisions (mirrors @spendguard/langchain D04 + @spendguard/sdk D05
// substrates):
//   - ESM-only output (review-standards.md §12.3).
//   - Node 20 target floor (peer-aligned with @spendguard/sdk).
//   - `splitting: false` keeps subpath entries as discrete files. Each
//     `entry` map key becomes a top-level `dist/<key>.js` + matching
//     `.d.ts`; `package.json#exports` keys must match.
import { defineConfig } from "tsup";

export default defineConfig({
  entry: {
    index: "src/index.ts",
    // SLICE 7 — function-reference alias for Mastra consumers. Same
    // factory under the Mastra-idiomatic name; review-standards §1.4 /
    // §1.6 enforces strict equality between
    // `createSpendGuardMiddleware` and `createSpendGuardLanguageMiddleware`.
    mastra: "src/mastra.ts",
  },
  format: ["esm"],
  dts: true,
  clean: true,
  sourcemap: false,
  // `splitting: true` is REQUIRED so that `index.ts` and `mastra.ts`
  // SHARE the underlying `middleware.ts` chunk at runtime instead of
  // each entry getting its own inlined copy of
  // `createSpendGuardMiddleware`. Without splitting, the bundler emits
  // two independent function declarations and the Mastra alias parity
  // assertion (`createSpendGuardLanguageMiddleware === createSpendGuardMiddleware`)
  // FAILS at runtime — verified by the SLICE 7 demo on first run.
  // Splitting moves the shared symbol into a chunk that both entries
  // import, preserving function-reference identity across the two
  // import paths. The SLICE 8 dist size is still well under the 50 KB
  // budget with splitting enabled.
  splitting: true,
  target: "node20",
  treeshake: true,
});
