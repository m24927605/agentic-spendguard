// tsup build configuration for @spendguard/mastra.
//
// Locked decisions (D38 design.md §11.14 + implementation.md §7):
//   - ESM-only output; single barrel entry; NO CJS artifact.
//   - `external: ["@mastra/core", "@spendguard/sdk"]` — both are peers and
//     must never be inlined into dist/index.js (hash-reuse + tree-shake
//     gates assert no substrate copy in the bundle).
//   - Node 22 target (engines floor `>=22.13.0` — the Mastra 1.x
//     requirement; do NOT harmonize down to the D04/D06 node20 target).
//   - `minify: true` — the implementation.md §2 bundle budget (≤ 40 KB min /
//     ≤ 12 KB gz) is defined on the MINIFIED dist/index.js, enforced as a
//     build failure by scripts/size-budget.sh.
import { defineConfig } from "tsup";

export default defineConfig({
  entry: { index: "src/index.ts" },
  format: ["esm"],
  dts: true,
  clean: true,
  sourcemap: false,
  splitting: false,
  target: "node22",
  treeshake: true,
  minify: true,
  external: ["@mastra/core", "@spendguard/sdk"],
});
