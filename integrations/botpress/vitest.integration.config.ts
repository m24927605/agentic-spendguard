// Integration-tier vitest config for @spendguard/botpress-integration.
//
// review-standards.md §4.1 / §4.7: the integration tier is path-filter-gated
// in CI (.github/workflows/botpress-integration-ci.yml) and excluded from the
// default `pnpm test` runner. It picks up only `tests/integration-*.test.ts`.
// Tests boot a real Botpress v12 container via testcontainers-node (when
// available) or — when testcontainers / Docker is not present — degrade to a
// lighter in-process emulation so the suite still runs in environments
// without a Docker daemon. The lighter emulation exercises the same hook
// dispatch surface the integration package exposes; the heavy testcontainers
// boot is a CI-only opt-in.
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["tests/integration-*.test.ts"],
    pool: "forks",
    testTimeout: 120_000,
    hookTimeout: 180_000,
  },
});
