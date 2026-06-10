// vitest configuration for @spendguard/mastra.
//
// Mirrors the @spendguard/sdk / @spendguard/ag-ui runner shape (ESM, node
// env, forks pool) so cross-package test patterns stay uniform.
//
// Coverage thresholds are wired to the D38 tests.md §1 package floors
// (≥ 90 % statements / ≥ 85 % branches). The default `pnpm run test`
// (`vitest run`) does NOT pass `--coverage` in COV_D38_01 — the coverage
// gate is exercised once the real TP suite lands (COV_D38_02..04); the
// thresholds here pre-wire the floor so enabling `--coverage` enforces it.
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
        statements: 90,
        branches: 85,
      },
    },
  },
});
