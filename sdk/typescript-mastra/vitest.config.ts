// vitest configuration for @spendguard/mastra.
//
// Mirrors the @spendguard/sdk / @spendguard/ag-ui runner shape (ESM, node
// env, forks pool) so cross-package test patterns stay uniform.
//
// Coverage thresholds enforce the FULL D38 tests.md §1 floor table
// (COV_D38_04, per the COV_D38_01 R1 polish note):
//   - package overall   ≥ 90 % statements / ≥ 85 % branches
//   - processor.ts      ≥ 90 % statements / ≥ 85 % branches
//   - identity.ts / inflight.ts / flatten.ts / usage.ts
//                       100 % statements / ≥ 90 % branches
// `pnpm run test` runs `vitest run --coverage`, so every floor is an
// exit-code gate (TA-01).
//
// Floors only make sense for the FULL suite: single-file runs (e.g. gate
// A3.7's `pnpm run test tests/hashReuse.test.ts`) exercise a slice of src
// and would spuriously fail every threshold, so thresholds are disabled
// when a test-file filter is present on the CLI.
import { defineConfig } from "vitest/config";

const isFileFilteredRun = process.argv.some((arg) => arg.endsWith(".test.ts"));

export default defineConfig({
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
    pool: "forks",
    testTimeout: 10_000,
    hookTimeout: 10_000,
    coverage: {
      provider: "v8",
      reporter: ["text"],
      include: ["src/**/*.ts"],
      thresholds: isFileFilteredRun
        ? undefined
        : {
            statements: 90,
            branches: 85,
            // tests.md §1 per-module floors (glob keys are matched against
            // the covered file paths; the package floor above stays the
            // default).
            "src/processor.ts": { statements: 90, branches: 85 },
            "src/identity.ts": { statements: 100, branches: 90 },
            "src/inflight.ts": { statements: 100, branches: 90 },
            "src/flatten.ts": { statements: 100, branches: 90 },
            "src/usage.ts": { statements: 100, branches: 90 },
          },
    },
  },
});
