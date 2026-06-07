// tsup build for @spendguard/flowise-nodes.
//
// Flowise discovers each node file under `dist/nodes/` and inspects its
// default export for a `nodeClass`. We emit a discrete file per node so
// the loader's path scan keeps working — `splitting: false` is the
// invariant here.
//
// Locked decisions (design.md §5–§7 + implementation.md §3, §7):
//   - ESM-only output (Flowise 2.x is ESM-friendly under Node 20).
//   - Node 20 target floor (peer-aligned with @spendguard/sdk).
//   - `external` strips peers so the published artefact stays small.
//   - SVG icon is base64-embedded via the `.svg` loader so we ship one
//     JS file per node, not a separate asset path.
import { defineConfig } from "tsup";

export default defineConfig({
  entry: {
    index: "src/index.ts",
    "nodes/SpendGuardChatModelWrapper": "src/nodes/SpendGuardChatModelWrapper.ts",
  },
  format: ["esm"],
  dts: true,
  clean: true,
  sourcemap: false,
  splitting: false,
  target: "node20",
  treeshake: true,
  loader: {
    ".svg": "base64",
  },
  external: [
    "@spendguard/sdk",
    "@spendguard/langchain",
    "flowise-components",
    "@langchain/core",
  ],
});
