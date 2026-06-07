// vitest configuration for n8n-nodes-spendguard.
//
// Mirrors @spendguard/langchain / @spendguard/inngest-agent-kit so the
// cross-package test runner shape stays uniform.

import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
    exclude: ["tests/e2e/**", "node_modules/**", "dist/**"],
    pool: "forks",
    testTimeout: 10_000,
    hookTimeout: 10_000,
  },
});
