// SpendGuard SDK — computePromptHash tests (SLICE 6 / COV_S05_06).
//
// **Cross-language byte-equivalence P0 invariant (review-standards §1.5)**:
// the TS output for the fixtures below MUST equal the Python
// `spendguard.prompt_hash.compute(text, tenant_id)` output byte-for-byte.
// The fixtures are generated against the canonical Python implementation;
// any drift here breaks the audit-chain rule dedup gate.
//
// Fixture generation command (Python, reference implementation):
//   import importlib.util
//   spec = importlib.util.spec_from_file_location(
//       "prompt_hash", "sdk/python/src/spendguard/prompt_hash.py"
//   )
//   mod = importlib.util.module_from_spec(spec); spec.loader.exec_module(mod)
//   mod.compute("hello world", "00000000-0000-0000-0000-000000000001")
//
// Spec refs:
//   - design.md §4.8 LOCKED surface
//   - implementation.md §7
//   - review-standards.md §1.5 cross-language P0 gate
//   - tests.md §5.3 cross-language fixture matrix

import { describe, expect, it } from "vitest";

import { computePromptHash } from "../src/promptHash.js";

describe("computePromptHash — cross-language Python parity (P0 gate)", () => {
  // FX1: ASCII prompt, UUID tenant.
  it("FX1: 'hello world' + UUID tenant matches Python output", () => {
    const got = computePromptHash("hello world", "00000000-0000-0000-0000-000000000001");
    expect(got).toBe("5d55a1ebc9782455de0979780fd6cf686127dadcba580f230ddc3fea31516d0d");
  });

  // FX2: empty prompt — common edge case for cold-start retries.
  it("FX2: empty prompt + UUID tenant matches Python output", () => {
    const got = computePromptHash("", "00000000-0000-0000-0000-000000000001");
    expect(got).toBe("698e521970ba6005a5555a4dc63797488a0f673a3386adfe0410aa11c9b6757b");
  });

  // FX3: leading + trailing ASCII whitespace stripped before hashing.
  it("FX3: ASCII-whitespace-bordered prompt + non-UUID tenant matches Python", () => {
    const got = computePromptHash("  trim me  ", "tenant-abc");
    expect(got).toBe("d97fa1377ce7133eeae08a7b9d67eaf50f80edc917d4550720ac6e9fccbd89e4");
  });

  // FX4: multi-byte UTF-8 codepoints (CJK + ASCII punctuation).
  it("FX4: UTF-8 multi-byte prompt matches Python output", () => {
    const got = computePromptHash("Hello, 世界!", "00000000-0000-0000-0000-000000000042");
    expect(got).toBe("7caa95cf8b5b9118721f192d4998515655d570c40be02c4c8402a201c6e2f7e5");
  });

  // FX5: same prompt as FX1 with different tenant — produces different hash.
  it("FX5: same prompt + different tenant produces a different hash (cross-tenant separation)", () => {
    const a = computePromptHash("hello world", "00000000-0000-0000-0000-000000000001");
    const b = computePromptHash("hello world", "tenant-abc");
    expect(a).not.toBe(b);
    expect(b).toBe("cd26941adac1453237abf00ea45438164f9107221e4ef2fbd3b520832c721d7a");
  });

  // FX6 / FX7: tenant UUID canonicalization — upper- and lowercase UUID
  // produce IDENTICAL hashes (canonicalizer lowercases) per privacy
  // canonicalization invariant in design.md §4.8.
  it("FX6/FX7: tenant UUID case is canonicalised (upper == lower)", () => {
    const upper = computePromptHash("hello world", "ABCDEF12-3456-7890-ABCD-EF1234567890");
    const lower = computePromptHash("hello world", "abcdef12-3456-7890-abcd-ef1234567890");
    expect(upper).toBe(lower);
    expect(upper).toBe("27ad8586fff06972454564d1fe1f447877a52807142815d2eb3e03f962152486");
  });
});

describe("computePromptHash — output shape", () => {
  it("always returns 64-char lowercase hex", () => {
    const out = computePromptHash("anything", "anything");
    expect(out).toMatch(/^[0-9a-f]{64}$/);
  });

  it("is deterministic — repeated calls return the same hash", () => {
    const a = computePromptHash("repeatable", "tenant-x");
    const b = computePromptHash("repeatable", "tenant-x");
    expect(a).toBe(b);
  });

  it("handles long prompts (8 KB+ arbitrary length)", () => {
    const long = "a".repeat(8192);
    const got = computePromptHash(long, "tenant-x");
    expect(got).toMatch(/^[0-9a-f]{64}$/);
  });

  it("trims ASCII whitespace but preserves internal whitespace", () => {
    // `"  hello world  "` and `"hello world"` MUST hash identically (leading
    // + trailing stripped).
    const trimmed = computePromptHash("hello world", "tenant-x");
    const padded = computePromptHash("  hello world  ", "tenant-x");
    expect(trimmed).toBe(padded);
  });

  it("does NOT trim Unicode whitespace (only ASCII)", () => {
    // U+00A0 NO-BREAK SPACE is NOT in the ASCII whitespace set Python uses.
    // The hash for the U+00A0-padded version MUST differ from the trimmed
    // baseline (whereas an ASCII-trim would collapse them).
    const ascii = computePromptHash("hello", "tenant-x");
    const unicodePadded = computePromptHash(" hello ", "tenant-x");
    expect(ascii).not.toBe(unicodePadded);
  });
});
