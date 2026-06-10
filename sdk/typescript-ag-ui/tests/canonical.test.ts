// TP-20..TP-24 — canonical JSON conformance (tests.md §4; design.md §7 LOCKED).
import { describe, expect, it } from "vitest";
import {
  AgUiEventValidationError,
  buildDecisionDenied,
  buildReservationCreated,
  canonicalEventJson,
} from "../src/index.js";
import type { SpendGuardAgUiEvent } from "../src/index.js";
import { CREATED_MAX, DENIED_MIN, TS_MS } from "./_support/vectors.js";

/** Helper: wrap an arbitrary value object in a CUSTOM envelope shape so the
 *  serializer constraints can be probed directly. */
function evtWith(value: Record<string, unknown>): SpendGuardAgUiEvent {
  return {
    type: "CUSTOM",
    name: "spendguard.budget.snapshot",
    value,
  };
}

describe("TP-20 recursive key sorting (§7.3)", () => {
  it("unsorted construction order serializes with recursively sorted keys", () => {
    const out = canonicalEventJson(
      evtWith({
        z_last: "1",
        a_first: { z_inner: "x", a_inner: "y", m_inner: { b: "2", a: "3" } },
        m_mid: ["keep", "caller", "order"],
      }),
    );
    expect(out).toBe(
      '{"name":"spendguard.budget.snapshot","type":"CUSTOM","value":{"a_first":{"a_inner":"y","m_inner":{"a":"3","b":"2"},"z_inner":"x"},"m_mid":["keep","caller","order"],"z_last":"1"}}',
    );
  });

  it("the envelope itself is key-sorted (name < timestamp < type < value)", () => {
    const out = canonicalEventJson({ ...evtWith({ a: "1" }), timestamp: TS_MS });
    expect(out).toBe(
      `{"name":"spendguard.budget.snapshot","timestamp":${TS_MS},"type":"CUSTOM","value":{"a":"1"}}`,
    );
  });

  it("objects nested inside arrays are sorted too", () => {
    const out = canonicalEventJson(evtWith({ arr: [{ z: "1", a: "2" }] }));
    expect(out).toContain('"arr":[{"a":"2","z":"1"}]');
  });
});

describe("TP-21 separators / whitespace / encoding (§7.1-§7.2)", () => {
  it('no ": ", no ", ", no newline, no trailing whitespace', () => {
    const out = canonicalEventJson(buildReservationCreated(CREATED_MAX, { timestampMs: TS_MS }));
    expect(out.includes('": "')).toBe(false);
    expect(out.includes('", "')).toBe(false);
    expect(out.includes(", ")).toBe(false);
    expect(out.includes("\n")).toBe(false);
    expect(out).toBe(out.trim());
  });

  it("bytes are UTF-8 without BOM", () => {
    const out = canonicalEventJson(buildReservationCreated(CREATED_MAX));
    expect(out.charCodeAt(0)).not.toBe(0xfeff);
    const bytes = Buffer.from(out, "utf8");
    expect(bytes[0]).toBe("{".charCodeAt(0));
    expect(bytes.toString("utf8")).toBe(out);
  });
});

describe("TP-22 Unicode passthrough + escape-set parity (§7.5)", () => {
  it("CJK / emoji / astral-plane chars serialize as raw UTF-8, not \\uXXXX", () => {
    const evt = buildDecisionDenied({
      ...DENIED_MIN,
      reasonCodes: ["預算已拒絕", "💸", "\u{10348}"],
    });
    const out = canonicalEventJson(evt);
    expect(out).toContain("預算已拒絕");
    expect(out).toContain("💸");
    expect(out).toContain("\u{10348}");
    expect(out.includes("\\u")).toBe(false);
  });

  it("control chars escape identically to Python (\\n shorthand, \\u001f otherwise)", () => {
    const out = canonicalEventJson(evtWith({ a: "line\nbreak", b: "ctl\u001fchar" }));
    expect(out).toContain('"line\\nbreak"');
    expect(out).toContain('"ctl\\u001fchar"');
    // The raw control bytes never pass through unescaped.
    expect(out.includes("\u001f")).toBe(false);
  });
});

describe("TP-23 rejection set (§7.4-§7.5)", () => {
  const rejections: ReadonlyArray<[string, Record<string, unknown>]> = [
    ["float value", { x: 1.5 }],
    ["NaN", { x: Number.NaN }],
    ["Infinity", { x: Number.POSITIVE_INFINITY }],
    ["-0", { x: -0 }],
    ["int > 2^53-1", { x: 2 ** 53 }],
    ["null value", { x: null }],
    ["non-ASCII object key", { 金額: "1" }],
    ["unpaired surrogate string", { x: "\uD800" }],
    ["undefined value", { x: undefined }],
  ];

  it.each(rejections)("%s throws AgUiEventValidationError", (_label, value) => {
    expect(() => canonicalEventJson(evtWith(value))).toThrowError(AgUiEventValidationError);
  });

  it("legal edge values pass: 2^53-1, negative int, booleans", () => {
    const out = canonicalEventJson(evtWith({ max: 2 ** 53 - 1, neg: -42, t: true, f: false }));
    expect(out).toContain(`"max":${2 ** 53 - 1}`);
    expect(out).toContain('"neg":-42');
    expect(out).toContain('"t":true');
    expect(out).toContain('"f":false');
  });
});

describe("TP-24 idempotence", () => {
  it("parse → canonicalize of its own output is byte-identical", () => {
    const evt = buildDecisionDenied(
      {
        ...DENIED_MIN,
        reasonCodes: ["預算已拒絕", "💸", "BUDGET_EXHAUSTED"],
        matchedRuleIds: ["rule\u001fctl"],
      },
      { timestampMs: TS_MS },
    );
    const once = canonicalEventJson(evt);
    const twice = canonicalEventJson(JSON.parse(once) as SpendGuardAgUiEvent);
    expect(twice).toBe(once);
  });
});
