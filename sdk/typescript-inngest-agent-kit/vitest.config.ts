// vitest configuration for @spendguard/inngest-agent-kit.
//
// Mirrors @spendguard/sdk's runner shape so cross-package test patterns stay
// uniform.
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
