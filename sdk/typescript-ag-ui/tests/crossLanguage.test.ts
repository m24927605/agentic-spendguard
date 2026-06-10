// TP-27 — cross-language byte-equivalence against the frozen corpus
// sdk/fixtures/cross-language/ag_ui_v1.json (tests.md §6, P0).
//
// The corpus is minted once by sdk/fixtures/cross-language/generate_ag_ui.mjs
// (slice COV_D39_01) and FROZEN: the Python suite (slice COV_D39_02) consumes
// the SAME file byte-for-byte. Never edit it in place — new vectors mint
// ag_ui_v2.json (D05 corpus discipline).
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  buildBudgetSnapshot,
  buildDecisionDenied,
  buildReservationCommitted,
  buildReservationCreated,
  buildReservationReleased,
  canonicalEventJson,
  encodeSse,
} from "../src/index.js";
import type { BuildContext, SpendGuardAgUiEvent } from "../src/index.js";

interface CorpusFixture {
  id: string;
  builder: string;
  description?: string;
  inputs: Record<string, unknown>;
  timestamp_ms?: number;
  expected_canonical_json: string;
  expected_sse: string;
}

interface Corpus {
  version: number;
  generated_at: string;
  generated_with: { package: string; version: string };
  fixtures: CorpusFixture[];
}

const CORPUS_PATH = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../../fixtures/cross-language/ag_ui_v1.json",
);

const corpus = JSON.parse(readFileSync(CORPUS_PATH, "utf8")) as Corpus;

const BUILDERS: Record<string, (input: never, ctx?: BuildContext) => SpendGuardAgUiEvent> = {
  buildBudgetSnapshot: buildBudgetSnapshot as never,
  buildReservationCreated: buildReservationCreated as never,
  buildReservationCommitted: buildReservationCommitted as never,
  buildReservationReleased: buildReservationReleased as never,
  buildDecisionDenied: buildDecisionDenied as never,
};

/** Corpus inputs are snake_case (the Python dataclass shape); the TS
 *  builders take camelCase (design.md §8.1). Mechanical key mapping only —
 *  values pass through untouched. */
function snakeToCamel(key: string): string {
  return key.replace(/_([a-z])/g, (_m, c: string) => c.toUpperCase());
}

function toCamelInputs(inputs: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(inputs)) {
    out[snakeToCamel(k)] = v;
  }
  return out;
}

describe("TP-27 corpus shape (acceptance A3.1 spot-checks)", () => {
  it("corpus has version 1 and >= 20 vectors", () => {
    expect(corpus.version).toBe(1);
    expect(corpus.generated_with.package).toBe("@spendguard/ag-ui");
    expect(corpus.fixtures.length).toBeGreaterThanOrEqual(20);
  });

  it("every builder is covered", () => {
    const used = new Set(corpus.fixtures.map((f) => f.builder));
    expect([...used].sort()).toEqual(Object.keys(BUILDERS).sort());
  });

  it("the tests.md §6 named matrix vectors exist", () => {
    // timestamp_ms: 0 pins "0 ≠ absent".
    expect(corpus.fixtures.some((f) => f.timestamp_ms === 0)).toBe(true);
    // One vector per denied_kind (5), incl. APPROVAL_REQUIRED + reason code.
    const kinds = new Set(
      corpus.fixtures
        .filter((f) => f.builder === "buildDecisionDenied")
        .map((f) => f.inputs.denied_kind),
    );
    expect([...kinds].sort()).toEqual([
      "APPROVAL_REQUIRED",
      "DENY",
      "SKIP",
      "STOP",
      "STOP_RUN_PROJECTION",
    ]);
    // One vector per committed outcome (4).
    const outcomes = new Set(
      corpus.fixtures
        .filter((f) => f.builder === "buildReservationCommitted")
        .map((f) => f.inputs.outcome),
    );
    expect([...outcomes].sort()).toEqual([
      "CLIENT_TIMEOUT",
      "PROVIDER_ERROR",
      "RUN_ABORTED",
      "SUCCESS",
    ]);
    // amount_atomic_observed vector exists.
    expect(
      corpus.fixtures.some(
        (f) =>
          f.builder === "buildReservationCommitted" &&
          f.inputs.amount_atomic_observed !== undefined,
      ),
    ).toBe(true);
    // 40-digit remaining_atomic vector exists.
    expect(
      corpus.fixtures.some(
        (f) =>
          typeof f.inputs.remaining_atomic === "string" && f.inputs.remaining_atomic.length === 40,
      ),
    ).toBe(true);
    // Unicode set: CJK + emoji + astral in reason_codes; U+001F in a
    // matched_rule_ids entry.
    expect(
      corpus.fixtures.some((f) => JSON.stringify(f.inputs.reason_codes ?? "").includes("💸")),
    ).toBe(true);
    expect(
      corpus.fixtures.some((f) =>
        (f.inputs.matched_rule_ids as string[] | undefined)?.some((r) => r.includes("\u001f")),
      ),
    ).toBe(true);
  });
});

describe("TP-27 TS == corpus, byte-for-byte", () => {
  it.each(corpus.fixtures.map((f) => [f.id, f] as const))("%s", (_id, fixture) => {
    const build = BUILDERS[fixture.builder];
    expect(build).toBeDefined();
    const ctx: BuildContext | undefined =
      fixture.timestamp_ms !== undefined ? { timestampMs: fixture.timestamp_ms } : undefined;
    const evt = (build as NonNullable<typeof build>)(toCamelInputs(fixture.inputs) as never, ctx);
    expect(canonicalEventJson(evt)).toBe(fixture.expected_canonical_json);
    expect(encodeSse(evt)).toBe(fixture.expected_sse);
  });
});
