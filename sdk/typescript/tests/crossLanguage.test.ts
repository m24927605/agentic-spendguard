// SpendGuard SDK — cross-language fixture harness (SLICE 9 / COV_S05_09).
//
// **P0 invariant — review-standards §2.1 / §2.2 / §2.5 + design.md §11**:
// the TS implementations of `computePromptHash`, `deriveIdempotencyKey`, and
// `deriveUuidFromSignature` MUST produce byte-identical output to the Python
// implementations for every fixture in `sdk/fixtures/cross-language/v1.json`.
//
// The fixture file is the SINGLE SOURCE OF TRUTH for cross-language parity.
// It is generated against the Python reference implementation
// (`sdk/fixtures/cross-language/generate.py`) and is consumed UNCHANGED by
// both the Python suite (`sdk/python/tests/test_cross_language_fixtures.py`)
// and this TS suite. Drift in either direction is a P0 review-standards §2
// blocker — see `sdk/fixtures/cross-language/README.md` for the
// add-a-fixture / mint-v2.json runbook.
//
// SLICES 6 + 7 shipped scattered FX1–FX7 fixtures inside `ids.test.ts` and
// `promptHash.test.ts`. Slice 9 consolidates them into v1.json (the FX1–FX7
// outputs are pinned identically here as FX1, FX2, FX3, FX4, FX5, FX6, FX7
// for `derive_idempotency_key` and FXP1, FXP2, FXP3, FXP4, FXP8 for
// `compute_prompt_hash`) and adds new FX8 + FXP5/FXP6/FXP7 + FXU3/FXU4
// vectors to hit the COV_S05_09 ≥20 volume floor. The scattered fixtures
// in the original test files remain in place for now — they exercise the
// same code paths under different test names. SLICE 10 may collapse them.
//
// Spec refs:
//   - docs/specs/coverage/D05_ts_sdk_substrate/design.md §11.
//   - docs/specs/coverage/D05_ts_sdk_substrate/review-standards.md §2
//     (P0 cross-language byte-equivalence gate).
//   - docs/slices/COV_S05_09_d05_cross_language_fixtures.md.
//
// Failure-mode contract: when a fixture drifts, the assertion error includes
// the fixture id + fn + canonicalised inputs so failures point at the exact
// mismatched vector. The test runner names every test
// `cross-language fixture ${id}: ${fn}` for the same reason.

import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  deriveIdempotencyKey,
  deriveUuidFromSignature,
} from "../src/ids.js";
import { computePromptHash } from "../src/promptHash.js";

// Resolve `sdk/fixtures/cross-language/v1.json` from this test file's
// location. The path is fixed-offset relative to `sdk/typescript/tests/`,
// not derived from cwd, so the suite passes whether vitest is invoked from
// the repo root or from `sdk/typescript/`.
const _here = path.dirname(fileURLToPath(import.meta.url));
const FIXTURES_PATH = path.resolve(
  _here,
  "..",
  "..",
  "fixtures",
  "cross-language",
  "v1.json",
);

interface Fixture {
  id: string;
  fn:
    | "derive_idempotency_key"
    | "compute_prompt_hash"
    | "derive_uuid_from_signature";
  description?: string;
  inputs: Record<string, unknown>;
  expected_output: string;
}

interface Corpus {
  version: number;
  generated_at?: string;
  fixtures: Fixture[];
}

function loadCorpus(): Corpus {
  const raw = fs.readFileSync(FIXTURES_PATH, "utf-8");
  return JSON.parse(raw) as Corpus;
}

function asString(v: unknown, ctx: string): string {
  if (typeof v !== "string") {
    throw new Error(`Fixture ${ctx} must be a string, got ${typeof v}`);
  }
  return v;
}

function evaluateFixture(f: Fixture): string {
  switch (f.fn) {
    case "derive_idempotency_key":
      return deriveIdempotencyKey({
        tenantId: asString(f.inputs.tenant_id, `${f.id}.tenant_id`),
        sessionId: asString(f.inputs.session_id, `${f.id}.session_id`),
        runId: asString(f.inputs.run_id, `${f.id}.run_id`),
        stepId: asString(f.inputs.step_id, `${f.id}.step_id`),
        llmCallId: asString(f.inputs.llm_call_id, `${f.id}.llm_call_id`),
        trigger: asString(f.inputs.trigger, `${f.id}.trigger`),
      });
    case "compute_prompt_hash":
      return computePromptHash(
        asString(f.inputs.prompt_text, `${f.id}.prompt_text`),
        asString(f.inputs.tenant_id, `${f.id}.tenant_id`),
      );
    case "derive_uuid_from_signature":
      return deriveUuidFromSignature(
        asString(f.inputs.signature, `${f.id}.signature`),
        { scope: asString(f.inputs.scope, `${f.id}.scope`) },
      );
    default: {
      // Exhaustiveness gate: unknown `fn` would mean v1.json grew a new
      // function the TS suite hasn't taught itself to dispatch yet.
      const exhaustive: never = f.fn;
      throw new Error(
        `Unknown cross-language fixture fn for ${
          (f as Fixture).id
        }: ${String(exhaustive)}. Update the TS harness to dispatch this fn or revert the v1.json change.`,
      );
    }
  }
}

const corpus = loadCorpus();

describe("cross-language parity v1 (P0 byte-equivalence gate)", () => {
  it("loads ≥20 fixtures from v1.json", () => {
    expect(corpus.version).toBe(1);
    expect(corpus.fixtures.length).toBeGreaterThanOrEqual(20);
  });

  it("covers all three locked cross-language functions", () => {
    const fns = new Set(corpus.fixtures.map((f) => f.fn));
    expect(fns).toContain("derive_idempotency_key");
    expect(fns).toContain("compute_prompt_hash");
    expect(fns).toContain("derive_uuid_from_signature");
  });

  it("has unique fixture ids (catches accidental dup-paste)", () => {
    const ids = corpus.fixtures.map((f) => f.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  for (const fixture of corpus.fixtures) {
    it(`cross-language fixture ${fixture.id}: ${fixture.fn}`, () => {
      const actual = evaluateFixture(fixture);
      // The custom message in the third arg surfaces the fixture id + the
      // canonicalised inputs in the vitest diff output. Cf. failure-mode
      // contract above + review-standards §2.7.
      if (actual !== fixture.expected_output) {
        throw new Error(
          `CROSS-LANGUAGE DRIFT for fixture ${fixture.id} (${fixture.fn}):\n` +
            `  inputs:   ${JSON.stringify(fixture.inputs)}\n` +
            `  expected: ${fixture.expected_output}\n` +
            `  actual:   ${actual}\n` +
            `TS implementation has diverged from Python. This is a P0 ` +
            `review-standards §2 blocker — drift here breaks audit-chain ` +
            `rule dedup and idempotency replay collapse.`,
        );
      }
      expect(actual).toBe(fixture.expected_output);
    });
  }
});

describe("cross-language parity v1 (canonicalisation invariants)", () => {
  // The fixture set encodes a specific canonicalisation invariant: mixed-
  // case UUID tenant IDs MUST hash identically to their lowercase form. The
  // fixture FXP8 carries the mixed-case input; this test re-runs the lower-
  // case form locally and asserts byte equality against the FXP8 expected
  // output. If FXP8 drifts independently of the lowercase form, EITHER the
  // canonicaliser is broken OR the fixture file was edited out-of-band.
  it("FXP8: mixed-case UUID tenant hash equals lowercase-tenant hash", () => {
    const fxp8 = corpus.fixtures.find((f) => f.id === "FXP8");
    expect(fxp8, "FXP8 fixture must exist in v1.json").toBeDefined();
    const tenant = asString(fxp8!.inputs.tenant_id, "FXP8.tenant_id");
    const prompt = asString(fxp8!.inputs.prompt_text, "FXP8.prompt_text");
    const lowered = computePromptHash(prompt, tenant.toLowerCase());
    expect(lowered).toBe(fxp8!.expected_output);
  });

  // FX5 (all-empty derive_idempotency_key) is the degraded-but-stable
  // contract. Two independent calls MUST produce the SAME output — and
  // that output MUST match the fixture (which the parametrised sweep above
  // already covers; this test additionally pins determinism).
  it("FX5: all-empty derive_idempotency_key is repeatable + matches fixture", () => {
    const fx5 = corpus.fixtures.find((f) => f.id === "FX5");
    expect(fx5).toBeDefined();
    const a = evaluateFixture(fx5!);
    const b = evaluateFixture(fx5!);
    expect(a).toBe(b);
    expect(a).toBe(fx5!.expected_output);
  });
});
