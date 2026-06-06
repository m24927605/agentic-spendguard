// SpendGuard SDK — embedded DEMO_PRICING snapshot tests (SLICE 6 / COV_S05_06).
//
// Coverage:
//   - DEMO_PRICING is importable via the `@spendguard/sdk/pricing/demo` subpath
//   - Snapshot covers ≥10 (provider, model) pairs from the seed YAML
//   - DEMO_PRICING_VERSION matches the seed YAML's `pricing_version` field
//     (when the YAML drifts, this gate fails and the snapshot must be regen'd)
//   - Specific seed entries produce the expected µUSD for representative calls
//
// Spec refs:
//   - design.md §4.9 DEMO_PRICING surface
//   - design.md §9.9 snapshot < 50 KB
//   - slices/COV_S05_06_d05_ids_prompt_hash_pricing.md test plan
//
// **Snapshot freshness gate**: parses `deploy/demo/init/pricing/seed.yaml`
// (sync read, no deps — line-by-line) and asserts the `pricing_version` field
// matches the TS constant. If the YAML drifts forward and this test fails,
// regenerate `src/pricing/demo.ts` (manual until SLICE 10 wires the script).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

import { PricingLookup } from "../src/pricing.js";
// Import from the source path under test — the build subpath
// `@spendguard/sdk/pricing/demo` resolves to this file at publish time.
import { DEMO_PRICING, DEMO_PRICING_VERSION } from "../src/pricing/demo.js";

describe("DEMO_PRICING — subpath snapshot", () => {
  it("is importable as a PricingLookup instance from the subpath", () => {
    expect(DEMO_PRICING).toBeInstanceOf(PricingLookup);
  });

  it("covers ≥10 distinct (provider, model) pairs", () => {
    // Sample the major models from the seed YAML.
    const expectedPairs: ReadonlyArray<readonly [string, string]> = [
      ["openai", "gpt-4o-mini"],
      ["openai", "gpt-4o"],
      ["openai", "o1"],
      ["openai", "o3-mini"],
      ["anthropic", "claude-haiku-4-5-20251001"],
      ["anthropic", "claude-sonnet-4-5-20250929"],
      ["anthropic", "claude-opus-4-7"],
      ["azure_openai", "gpt-4o-mini"],
      ["azure_openai", "gpt-4o"],
      ["bedrock", "anthropic.claude-haiku-4-5"],
      ["bedrock", "anthropic.claude-sonnet-4-5"],
      ["gemini", "gemini-2.0-flash"],
    ];
    expect(expectedPairs.length).toBeGreaterThanOrEqual(10);
    for (const [provider, model] of expectedPairs) {
      // Each pair MUST have at least one input or output price.
      const inputPrice = DEMO_PRICING.pricePerMillion(provider, model, "input");
      const outputPrice = DEMO_PRICING.pricePerMillion(provider, model, "output");
      expect(inputPrice !== null || outputPrice !== null).toBe(true);
    }
  });

  it("has DEMO_PRICING_VERSION matching deploy/demo/init/pricing/seed.yaml", () => {
    // Read the YAML by hand (no dep). We just need the `pricing_version: "v..."`
    // line — line-based parse is sufficient for the gate.
    const yamlPath = resolve(
      import.meta.dirname,
      "..",
      "..",
      "..",
      "deploy",
      "demo",
      "init",
      "pricing",
      "seed.yaml",
    );
    const yaml = readFileSync(yamlPath, "utf8");
    const match = yaml.match(/^pricing_version:\s*"([^"]+)"/m);
    expect(match).not.toBeNull();
    const yamlVersion = match![1]!;
    expect(DEMO_PRICING_VERSION).toBe(yamlVersion);
  });

  it("computes correct µUSD for openai gpt-4o-mini 1000/500 call", () => {
    // 1000 * $0.15/1M + 500 * $0.60/1M = 150 + 300 = 450 µUSD
    const got = DEMO_PRICING.usdMicrosForCall({
      provider: "openai",
      model: "gpt-4o-mini",
      inputTokens: 1000,
      outputTokens: 500,
    });
    expect(got).toBe(450);
  });

  it("computes correct µUSD for anthropic claude-haiku-4-5 1000/500 call", () => {
    // 1000 * $1.00/1M + 500 * $5.00/1M = 1000 + 2500 = 3500 µUSD
    const got = DEMO_PRICING.usdMicrosForCall({
      provider: "anthropic",
      model: "claude-haiku-4-5-20251001",
      inputTokens: 1000,
      outputTokens: 500,
    });
    expect(got).toBe(3500);
  });

  it("computes correct µUSD for gemini-2.0-flash 1000/500 call", () => {
    // 1000 * $0.10/1M + 500 * $0.40/1M = 100 + 200 = 300 µUSD
    const got = DEMO_PRICING.usdMicrosForCall({
      provider: "gemini",
      model: "gemini-2.0-flash",
      inputTokens: 1000,
      outputTokens: 500,
    });
    expect(got).toBe(300);
  });
});
