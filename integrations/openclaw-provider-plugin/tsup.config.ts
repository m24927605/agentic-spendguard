export default {
  entry: {
    index: "src/index.ts",
  },
  format: ["esm"],
  dts: true,
  clean: true,
  sourcemap: false,
  splitting: false,
  minify: false,
  bundle: true,
  target: "node22",
  outDir: "dist",
  external: ["@spendguard/sdk", "openclaw"],
};
