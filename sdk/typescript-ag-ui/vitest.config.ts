// vitest configuration for @spendguard/ag-ui.
//
// Mirrors the @spendguard/sdk runner shape (ESM, node env, forks pool) so
// cross-package test patterns stay uniform.
//
// Coverage floors are the tests.md §1 package floor (≥ 92 % statements /
// ≥ 88 % branches) enforced as thresholds — `npm run test` runs with
// `--coverage`, so a floor breach is a non-zero exit (acceptance A2.1).
// Single-file acceptance runs (A2.2-A2.4) invoke `npx vitest run tests/...`
// without `--coverage` and are not threshold-gated.
import { defineConfig } from "vitest/config";

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
      thresholds: {
        statements: 92,
        branches: 88,
      },
    },
  },
});
