// vitest configuration for @spendguard/langchain.
//
// Mirrors @spendguard/sdk's runner shape so cross-package test patterns stay
// uniform. SLICE 1 ships only the locked-surface smoke test; SLICE 2+ will
// add SpendGuardCallbackHandler unit + integration tests.
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
    pool: "forks",
    testTimeout: 10_000,
    hookTimeout: 10_000,
  },
});
