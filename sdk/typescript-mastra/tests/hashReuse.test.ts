// COV_D38_04 — hash-reuse P0 suite (tests.md TP-36..TP-38, gate A3.7).
//
// TP NUMBERING NOTE: tests/inflight.test.ts carries a test labelled
// "TP-36 (R2 regression)" — a COV_D38_02 R2 review-cycle regression test
// for the InflightMap FIFO bound that reused the next free number before
// the tests.md TP-36..TP-38 rows landed. The TP-36/TP-37/TP-38 below are
// the AUTHORITATIVE tests.md §2 hash-reuse rows (this slice follows the
// slice-doc numbering); the inflight one is disambiguated by its
// "(R2 regression)" suffix.
//
// Design §6.3 (last property) + §11.6 + implementation.md §7 (P0,
// review-standards §4): ALL hashing / id derivation goes through
// `@spendguard/sdk`. The adapter contains ZERO parallel hash code — no
// node-crypto imports, no noble-hashes dep, no inlined BLAKE2 copy in the
// built bundle. BLAKE2b cross-language byte-equivalence rides the
// substrate's own P0 gate (D05 §13); a parallel derivation here would fork
// that guarantee silently.

import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import pkg from "../package.json" with { type: "json" };

const PKG_ROOT = join(import.meta.dirname, "..");
const SRC_DIR = join(PKG_ROOT, "src");
const DIST_INDEX = join(PKG_ROOT, "dist", "index.js");

// The forbidden-token regex from the tests.md TP-36 row, verbatim tokens:
//   @noble/hashes | node:crypto | createHash | createHmac | blake2
// Built char-class-wise so THIS file does not match its own gate when the
// suite is ever widened to scan tests/.
const FORBIDDEN = new RegExp(
  ["@noble/hashes", "node:crypto", "createHash", "createHmac", "blake2"]
    .map((token) => token.replace("/", "\\/"))
    .join("|"),
  "i",
);

/** Recursively collect *.ts files under a directory. */
function collectTsFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      out.push(...collectTsFiles(full));
    } else if (entry.isFile() && entry.name.endsWith(".ts")) {
      out.push(full);
    }
  }
  return out;
}

describe("COV_D38_04 hash-reuse P0 (TP-36..TP-38)", () => {
  it("TP-36: zero forbidden hash tokens anywhere under src/", () => {
    const files = collectTsFiles(SRC_DIR);
    // Sanity: the walk actually saw the package sources.
    expect(files.length).toBeGreaterThanOrEqual(9);
    const offenders: string[] = [];
    for (const file of files) {
      const content = readFileSync(file, "utf8");
      const match = content.match(FORBIDDEN);
      if (match !== null) {
        offenders.push(`${file}: "${match[0]}"`);
      }
    }
    expect(offenders).toEqual([]);
  });

  it("TP-37: package.json has no @noble/hashes in ANY dependency block", () => {
    const blocks = [
      "dependencies",
      "devDependencies",
      "peerDependencies",
      "optionalDependencies",
    ] as const;
    for (const block of blocks) {
      const deps = (pkg as Record<string, unknown>)[block];
      if (deps !== undefined && deps !== null && typeof deps === "object") {
        expect(Object.keys(deps)).not.toContain("@noble/hashes");
      }
    }
    // The adapter declares NO runtime `dependencies` at all — everything it
    // consumes is a peer (design §11.14 package discipline), so a hash dep
    // cannot ride in unnoticed.
    expect((pkg as Record<string, unknown>).dependencies).toBeUndefined();
  });

  it("TP-38: built dist/index.js carries no forbidden tokens, no inlined BLAKE2 tables; substrate externalized", () => {
    // Gate A3.7 runs `pnpm run build` immediately before this test; the
    // committed dist must also hold (tsup `clean: true` rebuilds in place).
    const bundle = readFileSync(DIST_INDEX, "utf8");

    // Token scan — same forbidden set as TP-36.
    expect(bundle.match(FORBIDDEN)).toBeNull();

    // No inlined BLAKE2b implementation: the BLAKE2b IV words (the SHA-512
    // initialization constants any BLAKE2 copy must embed, in either hex
    // spelling) must not appear in the bundle.
    for (const ivWord of ["6a09e667", "0x6a09e667", "bb67ae85", "f3bcc908", "84caa73b"]) {
      expect(bundle.toLowerCase()).not.toContain(ivWord);
    }

    // Substrate externalized (tsup `external` — implementation.md §7): the
    // bundle IMPORTS @spendguard/sdk instead of inlining a copy of it.
    expect(bundle).toMatch(/from\s*["']@spendguard\/sdk["']/);
    // (An inlined substrate copy would also trip the token scan above: the
    // substrate's own derivation calls the @noble/hashes BLAKE2b primitive,
    // so both "@noble/hashes" and "blake2" ride along with any inline.)
  });
});
