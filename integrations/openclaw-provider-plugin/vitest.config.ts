import { fileURLToPath } from "node:url";

export default {
  resolve: {
    alias: {
      "@spendguard/sdk": fileURLToPath(new URL("../../sdk/typescript/dist/index.js", import.meta.url)),
    },
  },
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
    exclude: ["node_modules/**", "dist/**", "dist-tests/**"],
    pool: "forks",
    testTimeout: 10_000,
    hookTimeout: 10_000,
  },
};
