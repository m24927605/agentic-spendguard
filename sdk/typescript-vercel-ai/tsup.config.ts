// tsup build configuration for @spendguard/vercel-ai.
//
// SLICE 1 ships only the package skeleton — entry map is just the barrel.
// SLICE 2 will add the `createSpendGuardMiddleware` factory module; SLICE 5
// will add the `streaming` subpath; SLICE 7 will add the `mastra` subpath.
//
// Locked decisions (mirrors @spendguard/langchain D04 + @spendguard/sdk D05
// substrates):
//   - ESM-only output (review-standards.md §12.3).
//   - Node 20 target floor (peer-aligned with @spendguard/sdk).
//   - `splitting: false` keeps subpath entries as discrete files when added
//     in later slices.
import { defineConfig } from "tsup";

export default defineConfig({
  entry: { index: "src/index.ts" },
  format: ["esm"],
  dts: true,
  clean: true,
  sourcemap: false,
  splitting: false,
  target: "node20",
  treeshake: true,
});
