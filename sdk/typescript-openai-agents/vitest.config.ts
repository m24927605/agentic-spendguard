// vitest configuration for @spendguard/openai-agents.
//
// Mirrors @spendguard/vercel-ai (D06) + @spendguard/langchain (D04) +
// @spendguard/sdk (D05) runner shape so cross-package test patterns stay
// uniform. SLICE 1 + 2 ships the locked-surface smoke test + factory unit
// tests; SLICE 3+ will add cross-language signature fixture coverage and
// per-decision-outcome routing.
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
