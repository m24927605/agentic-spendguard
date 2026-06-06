// SpendGuard SDK — embedded demo pricing snapshot (SLICE 6 / COV_S05_06).
//
// Materialised from `deploy/demo/init/pricing/seed.yaml` as TS literals so
// adapters can `import { DEMO_PRICING } from "@spendguard/sdk/pricing/demo"`
// and call sidecar in dev without wiring the control-plane pricing fetch.
//
// Subpath-only: this module is NOT re-exported from the main `index.ts` barrel.
// Adapters that need pricing in dev import the subpath explicitly so the main
// bundle stays under the §10 120 KB budget.
//
// Snapshot version is pinned by `DEMO_PRICING_VERSION` matching the YAML's
// `pricing_version` field. The next release regenerates this file from the
// then-current `seed.yaml` (SLICE 10 dance — see slice doc).
//
// Spec refs:
//   - design.md §4.9 (DEMO_PRICING under `@spendguard/sdk/pricing/demo`)
//   - design.md §9.9 (snapshot < 50 KB)
//   - design.md §9.6 (regeneration cadence; release dance)
//
// To regenerate manually (until SLICE 10 wires the scripted refresh):
//   1. Edit `deploy/demo/init/pricing/seed.yaml`.
//   2. Update `pricing_version` constant + the `prices` Map below to match.
//   3. Run `pnpm run test tests/pricingDemo.test.ts` — the version assertion
//      gate fails if the constant drifts from the YAML.

import { PricingLookup } from "../pricing.js";

/**
 * Snapshot version pinned from `deploy/demo/init/pricing/seed.yaml`
 * `pricing_version` field. Adapters that mint receipts with `pricingVersion`
 * SHOULD prefer this constant so the demo wire matches the snapshot exactly.
 */
export const DEMO_PRICING_VERSION = "v2026.05.09-1";

/**
 * Demo pricing table — frozen at module load.
 *
 * Key format: `${provider}|${model}|${tokenKind}`. Values are USD per million
 * tokens. Materialised from `deploy/demo/init/pricing/seed.yaml`.
 *
 * Coverage (snapshot 2026.05.09):
 *   - OpenAI: gpt-4o-mini, gpt-4o, o1, o3-mini
 *   - Anthropic: claude-haiku-4-5-20251001, claude-sonnet-4-5-20250929, claude-opus-4-7
 *   - Azure OpenAI: gpt-4o-mini, gpt-4o
 *   - AWS Bedrock: anthropic.claude-haiku-4-5, anthropic.claude-sonnet-4-5
 *   - Google Gemini: gemini-2.0-flash
 *
 * Total 12 distinct (provider, model) pairs covering input / output /
 * cached_input / reasoning kinds per provider as the YAML lists them.
 */
const DEMO_PRICE_ENTRIES: ReadonlyArray<readonly [string, number]> = Object.freeze([
  // ============== OpenAI ==============
  ["openai|gpt-4o-mini|input", 0.15],
  ["openai|gpt-4o-mini|cached_input", 0.075],
  ["openai|gpt-4o-mini|output", 0.6],
  ["openai|gpt-4o|input", 2.5],
  ["openai|gpt-4o|cached_input", 1.25],
  ["openai|gpt-4o|output", 10.0],
  ["openai|o1|input", 15.0],
  ["openai|o1|cached_input", 7.5],
  ["openai|o1|output", 60.0],
  ["openai|o1|reasoning", 60.0],
  ["openai|o3-mini|input", 1.1],
  ["openai|o3-mini|cached_input", 0.55],
  ["openai|o3-mini|output", 4.4],
  ["openai|o3-mini|reasoning", 4.4],
  // ============== Anthropic ==============
  ["anthropic|claude-haiku-4-5-20251001|input", 1.0],
  ["anthropic|claude-haiku-4-5-20251001|cached_input", 0.1],
  ["anthropic|claude-haiku-4-5-20251001|output", 5.0],
  ["anthropic|claude-sonnet-4-5-20250929|input", 3.0],
  ["anthropic|claude-sonnet-4-5-20250929|cached_input", 0.3],
  ["anthropic|claude-sonnet-4-5-20250929|output", 15.0],
  ["anthropic|claude-opus-4-7|input", 15.0],
  ["anthropic|claude-opus-4-7|cached_input", 1.5],
  ["anthropic|claude-opus-4-7|output", 75.0],
  // ============== Azure OpenAI ==============
  ["azure_openai|gpt-4o-mini|input", 0.15],
  ["azure_openai|gpt-4o-mini|cached_input", 0.075],
  ["azure_openai|gpt-4o-mini|output", 0.6],
  ["azure_openai|gpt-4o|input", 2.5],
  ["azure_openai|gpt-4o|output", 10.0],
  // ============== AWS Bedrock ==============
  ["bedrock|anthropic.claude-haiku-4-5|input", 1.0],
  ["bedrock|anthropic.claude-haiku-4-5|output", 5.0],
  ["bedrock|anthropic.claude-sonnet-4-5|input", 3.0],
  ["bedrock|anthropic.claude-sonnet-4-5|output", 15.0],
  // ============== Google Gemini ==============
  ["gemini|gemini-2.0-flash|input", 0.1],
  ["gemini|gemini-2.0-flash|cached_input", 0.025],
  ["gemini|gemini-2.0-flash|output", 0.4],
]);

/**
 * Demo `PricingLookup` instance — ready to call.
 *
 * Use in dev / examples to compute USD-micros without wiring a control-plane
 * pricing fetch. Source of truth: `deploy/demo/init/pricing/seed.yaml` at
 * pricing_version `v2026.05.09-1`.
 *
 * @example
 *   import { DEMO_PRICING } from "@spendguard/sdk/pricing/demo";
 *   const micros = DEMO_PRICING.usdMicrosForCall({
 *     provider: "openai", model: "gpt-4o-mini",
 *     inputTokens: 1000, outputTokens: 500,
 *   });
 *   // 150 + 300 = 450 µUSD
 */
export const DEMO_PRICING: PricingLookup = new PricingLookup(new Map(DEMO_PRICE_ENTRIES));
