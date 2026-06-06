// SpendGuard SDK — PricingLookup + computeUsdMicros tests (SLICE 6 / COV_S05_06).
//
// Coverage:
//   - usdMicrosForCall computes correct micros for known fixture (OpenAI gpt-4o-mini)
//   - per-kind fallback to default kind when token kind has no price
//   - round-up to nearest µUSD (never under-charge)
//   - zero-token returns zero (degenerate case)
//   - StaticPricingLookup.lookup returns undefined for unknown model
//   - pricePerMillion returns null for unknown (provider, model, kind)
//   - USD_MICROS_PER_USD constant matches 1_000_000
//   - Map round-trip with the LOCKED `${provider}|${model}|${kind}` key format
//
// Spec refs:
//   - design.md §4.9 LOCKED PricingLookup surface
//   - implementation.md §8
//   - tests.md §3.4 pricing computation matrix

import { describe, expect, it } from "vitest";

import { PricingLookup, USD_MICROS_PER_USD } from "../src/pricing.js";

describe("USD_MICROS_PER_USD", () => {
  it("equals 1_000_000 (one million micros per USD)", () => {
    expect(USD_MICROS_PER_USD).toBe(1_000_000);
  });
});

describe("PricingLookup.pricePerMillion()", () => {
  const table = new Map<string, number>([
    ["openai|gpt-4o-mini|input", 0.15],
    ["openai|gpt-4o-mini|output", 0.6],
  ]);
  const pricing = new PricingLookup(table);

  it("returns the configured $/1M-tokens for known (provider, model, kind)", () => {
    expect(pricing.pricePerMillion("openai", "gpt-4o-mini", "input")).toBe(0.15);
    expect(pricing.pricePerMillion("openai", "gpt-4o-mini", "output")).toBe(0.6);
  });

  it("returns null for unknown model", () => {
    expect(pricing.pricePerMillion("openai", "gpt-5", "input")).toBeNull();
  });

  it("returns null for unknown provider", () => {
    expect(pricing.pricePerMillion("nope", "gpt-4o-mini", "input")).toBeNull();
  });

  it("returns null for unknown token kind", () => {
    expect(pricing.pricePerMillion("openai", "gpt-4o-mini", "vision_input")).toBeNull();
  });
});

describe("PricingLookup.usdMicrosForCall()", () => {
  const table = new Map<string, number>([
    ["openai|gpt-4o-mini|input", 0.15],
    ["openai|gpt-4o-mini|output", 0.6],
    ["openai|gpt-4o-mini|cached_input", 0.075],
    ["anthropic|claude-haiku-4-5-20251001|input", 1.0],
    ["anthropic|claude-haiku-4-5-20251001|output", 5.0],
  ]);
  const pricing = new PricingLookup(table);

  it("computes correct micros for gpt-4o-mini 1000-input / 500-output call", () => {
    // 1000 * 0.15 / 1M = 0.00015 USD = 150 µUSD
    // 500 * 0.6 / 1M = 0.0003 USD = 300 µUSD
    // total = 450 µUSD
    const got = pricing.usdMicrosForCall({
      provider: "openai",
      model: "gpt-4o-mini",
      inputTokens: 1000,
      outputTokens: 500,
    });
    expect(got).toBe(450);
  });

  it("charges only configured kinds (input-only)", () => {
    // 1_000_000 * 0.15 / 1M = 0.15 USD = 150_000 µUSD
    const got = pricing.usdMicrosForCall({
      provider: "openai",
      model: "gpt-4o-mini",
      inputTokens: 1_000_000,
    });
    expect(got).toBe(150_000);
  });

  it("falls back to defaultKind=output when explicit kind has no price", () => {
    // Anthropic table lacks cached_input → falls back to output ($5/1M).
    // 1000 cached_input * $5/1M = 0.005 USD = 5000 µUSD
    const got = pricing.usdMicrosForCall({
      provider: "anthropic",
      model: "claude-haiku-4-5-20251001",
      cachedInputTokens: 1000,
    });
    expect(got).toBe(5000);
  });

  it("round-up: 1 input token at $0.15/1M = 0.00015 µUSD → 1 µUSD (never under-charge)", () => {
    // 1 * 0.15 / 1M = 0.00000015 USD = 0.15 µUSD → ceil → 1 µUSD
    const got = pricing.usdMicrosForCall({
      provider: "openai",
      model: "gpt-4o-mini",
      inputTokens: 1,
    });
    expect(got).toBe(1);
  });

  it("returns 0 µUSD when all token counts are 0", () => {
    const got = pricing.usdMicrosForCall({
      provider: "openai",
      model: "gpt-4o-mini",
      inputTokens: 0,
      outputTokens: 0,
    });
    expect(got).toBe(0);
  });

  it("ignores negative token counts (treated as 0)", () => {
    const got = pricing.usdMicrosForCall({
      provider: "openai",
      model: "gpt-4o-mini",
      inputTokens: -100,
      outputTokens: 500,
    });
    // Only the 500 output tokens charge: 500 * 0.6 / 1M = 300 µUSD
    expect(got).toBe(300);
  });

  it("returns 0 µUSD when model is unknown and no default fallback is available", () => {
    const got = pricing.usdMicrosForCall({
      provider: "fictional",
      model: "fictional-1",
      inputTokens: 1000,
      outputTokens: 500,
    });
    expect(got).toBe(0);
  });
});

describe("PricingLookup — constructor + defaultKind", () => {
  it("uses default 'output' kind when opts.defaultKind not specified", () => {
    const table = new Map<string, number>([["openai|gpt-4o-mini|output", 0.6]]);
    const pricing = new PricingLookup(table);
    // input has no entry — falls back to output ($0.6/1M).
    // 1000 * 0.6 / 1M = 600 µUSD
    const got = pricing.usdMicrosForCall({
      provider: "openai",
      model: "gpt-4o-mini",
      inputTokens: 1000,
    });
    expect(got).toBe(600);
  });

  it("uses custom defaultKind when specified", () => {
    const table = new Map<string, number>([["openai|gpt-4o-mini|input", 0.15]]);
    const pricing = new PricingLookup(table, { defaultKind: "input" });
    // output has no entry — falls back to input ($0.15/1M).
    const got = pricing.usdMicrosForCall({
      provider: "openai",
      model: "gpt-4o-mini",
      outputTokens: 1000,
    });
    expect(got).toBe(150);
  });
});
