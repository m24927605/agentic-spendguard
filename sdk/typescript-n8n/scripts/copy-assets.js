#!/usr/bin/env node
/**
 * Copy non-TS assets (icons, codex metadata JSON) from `nodes/` into
 * `dist/nodes/` after `tsc` runs. n8n's loader expects the runtime icon +
 * `.node.json` files next to the compiled `.node.js`.
 *
 * D37 SLICE 1 — replaces gulp from the implementation.md surface with a
 * zero-dep Node script. The behaviour is identical (recursive copy of
 * `*.svg` + `*.node.json`); the dep diet keeps the published tarball
 * under the 200 KB review-standards budget.
 */
const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const SRC = path.join(ROOT, "nodes");
const DEST = path.join(ROOT, "dist", "nodes");

const ASSET_EXTS = new Set([".svg", ".png"]);

function copyAssets(srcDir, destDir) {
  if (!fs.existsSync(srcDir)) return;
  fs.mkdirSync(destDir, { recursive: true });
  for (const entry of fs.readdirSync(srcDir, { withFileTypes: true })) {
    const srcPath = path.join(srcDir, entry.name);
    const destPath = path.join(destDir, entry.name);
    if (entry.isDirectory()) {
      copyAssets(srcPath, destPath);
      continue;
    }
    const ext = path.extname(entry.name);
    if (ASSET_EXTS.has(ext) || entry.name.endsWith(".node.json")) {
      fs.copyFileSync(srcPath, destPath);
      process.stdout.write(`[copy-assets] ${path.relative(ROOT, srcPath)} -> ${path.relative(ROOT, destPath)}\n`);
    }
  }
}

copyAssets(SRC, DEST);
