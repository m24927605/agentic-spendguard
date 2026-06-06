// tsup build configuration for @spendguard/sdk.
//
// This is a placeholder skeleton for COV_S05_01 — the real `entry` map (with
// every subpath export per design.md §4.1) is populated in COV_S05_10 once the
// source files exist. Locked decisions enforced here:
//
//   - ESM-only output (design.md §9.1, §6.1).
//   - Node 20 target floor (design.md §6.2).
//   - Source maps + .d.ts emission on.
//
// `tsup` is not installed in this slice; the manifest's `build` script is a
// no-op until COV_S05_02 wires the toolchain. This config file exists so
// subsequent slices have a known location to edit rather than create.
import { defineConfig } from "tsup";

export default defineConfig({
  entry: {
    // Populated in COV_S05_02+. Listed explicitly here as a forward-reference
    // contract so reviewers see the locked subpath shape from design.md §4.1.
    // index: "src/index.ts",
    // client: "src/client.ts",
    // errors: "src/errors.ts",
    // ids: "src/ids.ts",
    // pricing: "src/pricing.ts",
    // "pricing/demo": "src/pricing/demo.ts",
    // promptHash: "src/promptHash.ts",
    // runPlan: "src/runPlan.ts",
    // "_proto/index": "src/_proto/index.ts",
  },
  format: ["esm"],
  dts: true,
  splitting: false,
  sourcemap: true,
  clean: true,
  target: "node20",
  treeshake: true,
  // No CJS shim. See design.md §9.1 — dual-package hazard with @grpc/grpc-js
  // is well-documented; CJS Node ≤ 18 is explicitly unsupported.
});
