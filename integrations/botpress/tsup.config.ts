// tsup build configuration for @spendguard/botpress-integration.
//
// SLICE 1 ships the package skeleton; downstream slices add hooks, reservation
// delegate, adapters and lifecycle. Mirrors the
// @spendguard/inngest-agent-kit / @spendguard/openai-agents / @spendguard/sdk
// tsup config — ESM-only, Node 20 target floor, peer-aligned with
// @spendguard/sdk + @botpress/sdk.
//
// Locked decisions (design.md §5 + review-standards.md §1.4):
//   - ESM-only output.
//   - Node 20 target floor (peer-aligned with @spendguard/sdk).
//   - `splitting: false` keeps the single barrel as a discrete file.
//   - `treeshake: true` strips any unreferenced imports.
//   - `external: ["@spendguard/sdk", "@botpress/sdk"]` keeps both peer deps
//     OUT of the bundle so the published artefact is < 100 KB.
//   - `noExternal: ["zod"]` would bundle zod, but Botpress re-exports zod from
//     `@botpress/sdk` so we leave zod as a runtime-resolved import that
//     resolves through `@botpress/sdk`'s peer.
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
  external: ["@spendguard/sdk", "@botpress/sdk", "zod"],
});
