// vitest configuration for @spendguard/botpress-integration (unit tier).
//
// Mirrors @spendguard/inngest-agent-kit's runner shape. The unit suite uses
// an in-process HTTP server (see tests/_mockSidecar.ts) that emulates the D09
// SLICE 1 HTTP companion endpoints `/v1/decision` and `/v1/trace`. No Docker,
// no real Botpress runtime — those live in the integration tier (see
// vitest.integration.config.ts).
//
// review-standards.md §4.7: the default `pnpm test` MUST NOT pick up the
// integration tier (`tests/integration-v12.test.ts`). The `include` glob
// excludes anything matching `integration-*.test.ts`.
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
    exclude: ["tests/integration-*.test.ts", "node_modules/**", "dist/**"],
    pool: "forks",
    testTimeout: 10_000,
    hookTimeout: 10_000,
  },
});
