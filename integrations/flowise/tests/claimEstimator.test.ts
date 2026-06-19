// claimEstimator tests — covers acceptance.md A2.4 (CE-01..CE-07).
//
// The estimator is the no-code escape hatch — the JSON input on the
// canvas drives every reservation's atomic claim. Get this wrong and
// either every conversation overpays or every conversation under-claims.

import { describe, expect, it } from "vitest";

import {
  DEFAULT_CLAIM_ATOMIC,
  DEFAULT_CLAIM_SCOPE,
  buildClaimEstimator,
} from "../src/claimEstimator.js";

describe("buildClaimEstimator", () => {
  it("CE-01 default — empty string yields conservative $1 USD-micros claim", () => {
    const fn = buildClaimEstimator({ json: "", unit: "usd_micros" });
    const claims = fn();
    expect(claims).toHaveLength(1);
    expect(claims[0]).toEqual({
      scopeId: DEFAULT_CLAIM_SCOPE,
      amountAtomic: DEFAULT_CLAIM_ATOMIC,
      unit: "usd_micros",
    });
  });

  it("CE-02 default — whitespace-only JSON still uses the conservative default", () => {
    const fn = buildClaimEstimator({ json: "   \n\t", unit: "usd_micros" });
    expect(fn()).toEqual([
      { scopeId: DEFAULT_CLAIM_SCOPE, amountAtomic: DEFAULT_CLAIM_ATOMIC, unit: "usd_micros" },
    ]);
  });

  it("CE-03 override — full JSON object propagates amountAtomic + scopeId + unit", () => {
    const json = JSON.stringify({
      amountAtomic: "500000",
      scopeId: "premium-tier",
      unit: "anthropic_credits",
    });
    const fn = buildClaimEstimator({ json, unit: "usd_micros" });
    expect(fn()).toEqual([
      {
        scopeId: "premium-tier",
        amountAtomic: "500000",
        unit: "anthropic_credits",
      },
    ]);
  });

  it("CE-04 override — missing scopeId falls back to DEFAULT_CLAIM_SCOPE", () => {
    const fn = buildClaimEstimator({
      json: JSON.stringify({ amountAtomic: "2500000" }),
      unit: "usd_micros",
    });
    expect(fn()).toEqual([
      { scopeId: DEFAULT_CLAIM_SCOPE, amountAtomic: "2500000", unit: "usd_micros" },
    ]);
  });

  it("CE-05 error — missing amountAtomic throws a deterministic message", () => {
    expect(() =>
      buildClaimEstimator({
        json: JSON.stringify({ scopeId: "x" }),
        unit: "usd_micros",
      }),
    ).toThrowError(/must include 'amountAtomic' as a decimal string/);
  });

  it("CE-06 error — non-decimal amountAtomic is rejected explicitly", () => {
    expect(() =>
      buildClaimEstimator({
        json: JSON.stringify({ amountAtomic: "10e6" }),
        unit: "usd_micros",
      }),
    ).toThrowError(/must be a decimal string/);
    expect(() =>
      buildClaimEstimator({
        json: JSON.stringify({ amountAtomic: "abc" }),
        unit: "usd_micros",
      }),
    ).toThrowError(/must be a decimal string/);
  });

  it("CE-07 error — invalid JSON surface includes 'is not valid JSON'", () => {
    expect(() => buildClaimEstimator({ json: "{not-json}", unit: "usd_micros" })).toThrowError(
      /claimEstimatorJson is not valid JSON/,
    );
  });

  it("CE-bonus — JSON arrays / primitives are rejected with a clear message", () => {
    expect(() => buildClaimEstimator({ json: "42", unit: "usd_micros" })).toThrowError(
      /claimEstimatorJson must be a JSON object/,
    );
    expect(() =>
      buildClaimEstimator({ json: JSON.stringify(["a"]), unit: "usd_micros" }),
    ).toThrowError(/must include 'amountAtomic'/);
  });

  it("CE-bonus2 — repeated invocation of the same estimator returns equal claims (purity)", () => {
    const fn = buildClaimEstimator({
      json: JSON.stringify({ amountAtomic: "12345" }),
      unit: "usd_micros",
    });
    expect(fn()).toEqual(fn());
  });
});
