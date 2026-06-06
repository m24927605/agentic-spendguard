// tsup build configuration for @spendguard/sdk.
//
// SLICE 3 (`docs/slices/COV_S05_03_d05_client_skeleton.md`) populates the
// entry map for the modules its scope adds:
//   - index (full barrel)
//   - client / config / errors / env / version (subpath leaves)
//   - _proto/index (proto barrel; the _proto/spendguard/** leaves are NOT
//     listed as standalone entry points because they're consumed transitively
//     through the barrel and listing them would explode the build matrix).
//
// SLICE 6-7 will add `ids`, `pricing`, `pricing/demo`, `promptHash`, `runPlan`
// per design.md §4.1 subpath table; SLICE 10 finalises the size assertion.
//
// Locked decisions enforced here:
//   - ESM-only output (design.md §9.1, §6.1).
//   - Node 20 target floor (design.md §6.2).
//   - Source maps + .d.ts emission on.
//   - `splitting: false` keeps subpath entries as discrete files.
//
// `tsup` is installed as a devDependency starting in COV_S05_03.
import { defineConfig } from "tsup";

export default defineConfig({
  entry: {
    // Subpath entries match design.md §4.1 LOCKED subpath table. SLICE 3
    // ships the four that this slice's source code populates; SLICE 6-7 will
    // add the rest (ids, pricing, pricing/demo, promptHash, runPlan) when
    // their source files land.
    //
    // Internal modules (config, env, version) are NOT shipped as standalone
    // subpaths — they're consumed transitively through the `index` barrel.
    // The dist/*.js artefacts still exist for `client.ts` to import them at
    // runtime via tsup's intra-bundle resolution.
    index: "src/index.ts",
    client: "src/client.ts",
    errors: "src/errors.ts",
    proto: "src/proto.ts",
    // SLICE 6 entries (COV_S05_06):
    ids: "src/ids.ts",
    pricing: "src/pricing.ts",
    "pricing/demo": "src/pricing/demo.ts",
    promptHash: "src/promptHash.ts",
    // SLICE 7 (COV_S05_07) entry — `withRunPlan` + `currentRunPlan` substrate.
    runPlan: "src/runPlan.ts",
    // SLICE 8 (COV_S05_08) entries — OTel + retry + idempotency cache.
    otel: "src/otel.ts",
    retry: "src/retry.ts",
    cache: "src/cache.ts",
  },
  format: ["esm"],
  // `resolve: true` inlines cross-entry type re-exports rather than spilling
  // chunk files like `client-XXXX.d.ts`. Each subpath ends up with a fully
  // self-contained `.d.ts` so consumers don't trip over hashed chunk imports.
  dts: { resolve: true },
  splitting: false,
  sourcemap: true,
  clean: true,
  target: "node20",
  treeshake: true,
  // No CJS shim. See design.md §9.1 — dual-package hazard with @grpc/grpc-js
  // is well-documented; CJS Node ≤ 18 is explicitly unsupported.
  // ESM extension: tsup defaults to `.js` for ESM; with `"type": "module"`
  // in package.json this resolves correctly via Node's ESM resolver.
});
