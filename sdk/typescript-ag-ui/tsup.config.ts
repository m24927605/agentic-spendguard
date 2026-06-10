// tsup build configuration for @spendguard/ag-ui.
//
// Locked decisions (mirrors the @spendguard/sdk / @spendguard/langchain
// substrate pattern, COV_D39_01):
//   - ESM-only output; single barrel entry (design.md §8.1 — nothing else).
//   - Node 20 target floor (engines-aligned with the other @spendguard
//     packages); no `node:` imports anywhere in src/, so the bundle is
//     browser-safe (tests.md TP-30).
//   - `minify: true` — the implementation.md §3 bundle budget is defined on
//     the MINIFIED dist/index.js (≤ 8 KB min / ≤ 3 KB gz), enforced as a
//     build failure by scripts/size-budget.sh.
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
  minify: true,
});
