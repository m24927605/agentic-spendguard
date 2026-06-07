// tsup build configuration for @spendguard/inngest-agent-kit.
//
// SLICE 1+2+3 bundle ships the factory wrap + identity helpers + extract
// helpers. Mirrors the @spendguard/langchain / @spendguard/openai-agents
// tsup config — ESM-only, Node 20 target floor, peer-aligned with
// @spendguard/sdk and @inngest/agent-kit.
//
// Locked decisions:
//   - ESM-only output.
//   - Node 20 target floor (peer-aligned with @spendguard/sdk).
//   - `splitting: false` keeps the single barrel as a discrete file.
//   - `treeshake: true` strips any unreferenced imports from @spendguard/sdk
//     so the bundle stays under the 35 KB / 10 KB budget.
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
