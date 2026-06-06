// vitest configuration for @spendguard/sdk.
//
// Placeholder skeleton for COV_S05_01. The test suite itself ships in
// COV_S05_09 (`tests/**/*.test.ts`); this config locks the runner shape so
// later slices do not re-litigate it.
//
// Locked choices (design.md §6.1):
//   - Vitest 2.x — chosen over node:test for cross-language fixture diff
//     APIs and Bun/Deno compat shim.
//   - ESM, Node environment, isolated per-file workers via "forks" pool —
//     the COV_S05_09 mock UDS sidecar needs distinct sockets per worker.
//   - Timeouts bumped from the 5s default since cold gRPC channel setup
//     can take 2–3s on contended CI.
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
    // pool=forks: each test file runs in its own Node process so the mock
    // UDS sidecar (COV_S05_09) can bind a unique socket path without
    // collision.
    pool: "forks",
    testTimeout: 10_000,
    hookTimeout: 10_000,
    coverage: {
      provider: "v8",
      reporter: ["text", "lcov"],
    },
  },
});
