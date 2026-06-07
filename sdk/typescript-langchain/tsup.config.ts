// tsup build configuration for @spendguard/langchain.
//
// SLICE 1 (`docs/slices/COV_D04_S1_pkg_init.md`) ships only the package
// skeleton — entry map is just the barrel. SLICE 2 will add the
// SpendGuardCallbackHandler module.
//
// Locked decisions (mirrors @spendguard/sdk D05 substrate):
//   - ESM-only output.
//   - Node 20 target floor (peer-aligned with @spendguard/sdk).
//   - `splitting: false` keeps subpath entries as discrete files.
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
