export default {
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
    exclude: ["node_modules/**", "dist/**", "dist-tests/**"],
    pool: "forks",
    testTimeout: 10_000,
    hookTimeout: 10_000,
  },
};
