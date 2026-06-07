// vitest configuration for @spendguard/flowise-nodes (unit tier).
//
// Mirrors @spendguard/botpress-integration's runner shape. The unit suite
// uses an in-process mock sidecar (see tests/_support/mockSidecar.ts) that
// emulates the D09 HTTP companion endpoints `/v1/decision` and `/v1/trace`.
// No Docker; the testcontainers-based E2E suite lives under tests/e2e/ and
// is gated behind `D35_E2E=1` per acceptance.md A2.6.
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
