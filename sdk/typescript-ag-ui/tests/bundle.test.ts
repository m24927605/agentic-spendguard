// TP-30..TP-31 — packaging hygiene (tests.md §8).
//
// Requires `pnpm run build` to have produced dist/index.js (acceptance gate
// order: A1.4 build precedes A2.x tests).
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { describe, expect, it } from "vitest";

const DIST = resolve(dirname(fileURLToPath(import.meta.url)), "../dist/index.js");

function dist(): string {
  if (!existsSync(DIST)) {
    throw new Error("dist/index.js missing — run `pnpm run build` before the test suite");
  }
  return readFileSync(DIST, "utf8");
}

describe("TP-30 zero-dep, browser-safe bundle", () => {
  it("dist/index.js contains no node: import", () => {
    expect(dist().includes("node:")).toBe(false);
  });

  it("dist/index.js contains no require(", () => {
    expect(dist().includes("require(")).toBe(false);
  });

  it("dist/index.js contains no @ag-ui/core or @spendguard/sdk import", () => {
    expect(dist().includes("@ag-ui/core")).toBe(false);
    expect(dist().includes("@spendguard/sdk")).toBe(false);
  });
});

describe("TP-31 size budget (implementation.md §3)", () => {
  it("minified dist/index.js <= 8 KB", () => {
    const bytes = Buffer.byteLength(dist(), "utf8");
    expect(bytes).toBeLessThanOrEqual(8 * 1024);
  });

  it("gzipped dist/index.js <= 3 KB", () => {
    const gz = gzipSync(Buffer.from(dist(), "utf8")).length;
    expect(gz).toBeLessThanOrEqual(3 * 1024);
  });
});
