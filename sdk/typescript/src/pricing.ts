// SpendGuard SDK — pricing lookup + USD-micros computation (SLICE 6 /
// COV_S05_06).
//
// Mirror of `sdk/python/src/spendguard/pricing.py`. Adapters that want to
// charge an LLM call against a USD-denominated budget convert
// (provider, model, token_counts) into USD-micros here.
//
// For production, the pricing table comes from the canonical pricing-table
// keyed by the bundle's `pricing_version` (forward-reserved; sidecar fetch
// not in v0.1.x). POC sidecar adapters hardcode a subset; the demo snapshot
// ships under `@spendguard/sdk/pricing/demo` as the embedded `DEMO_PRICING`
// `PricingLookup` instance.
//
// Spec refs:
//   - design.md §4.9 (LOCKED `PricingLookup` surface)
//   - implementation.md §8 (`src/pricing.ts`)
//   - tests.md §3.4 (pricing computation matrix)

import { PricingMissingError } from "./errors.js";

/** USD micros per USD — 1 USD = 1,000,000 µUSD. */
export const USD_MICROS_PER_USD = 1_000_000;

/**
 * Composite lookup key for the pricing table.
 *
 * `(provider, model, tokenKind)` triples key the `PriceTable`. `tokenKind`
 * is one of `"input"`, `"output"`, `"cached_input"`, `"vision_input"`,
 * `"audio_input"`, `"reasoning"`.
 *
 * The `PriceTable` Map serializes the triple to a `${provider}|${model}|${kind}`
 * string — JS Map keys do not support tuple identity, so we flatten.
 */
export type PriceKey = readonly [provider: string, model: string, tokenKind: string];

/**
 * Frozen-at-construction pricing table.
 *
 * Keys: `${provider}|${model}|${tokenKind}` strings.
 * Values: USD price per million tokens (e.g. `0.15` = $0.15 / 1M tokens).
 */
export type PriceTable = ReadonlyMap<string, number>;

/**
 * Frozen pricing table → USD-micros computation.
 *
 * The lookup is not side-effecting — there is no DB query, no network call.
 * Callers fetch + cache pricing once (e.g., at handshake time) and pass it in.
 * Mirrors Python `PricingLookup` semantics including:
 *   - default kind fallback (typically `"output"`) when a token kind has no
 *     configured price.
 *   - per-kind charging for input / output / cached_input buckets.
 *   - round-up to the nearest µUSD so the customer is never under-charged.
 */
export class PricingLookup {
  private readonly table: PriceTable;
  private readonly defaultKind: string;

  constructor(table: PriceTable, opts?: { defaultKind?: string }) {
    this.table = table;
    this.defaultKind = opts?.defaultKind ?? "output";
  }

  /** Return $/1M-tokens or `null` if `(provider, model, kind)` is missing. */
  pricePerMillion(provider: string, model: string, tokenKind: string): number | null {
    const v = this.table.get(`${provider}|${model}|${tokenKind}`);
    return v === undefined ? null : v;
  }

  /**
   * Compute the µUSD cost of a single LLM call.
   *
   * Charges per-kind when prices are available; falls back to the default
   * kind (typically `"output"`) for any token bucket without a configured
   * price. Result is rounded UP to the nearest µUSD (fail-safe for the
   * customer — never under-charge due to FP truncation). The minimum
   * returned value is `1` (we never claim zero cost on a non-zero token
   * count).
   *
   * Fail-closed: when a token bucket has a non-zero count but NEITHER the
   * specific kind NOR the default kind has a configured price, this throws
   * {@link PricingMissingError} instead of silently charging $0. Coercing a
   * missing price to 0 would under-count the budget — the exact under-charge
   * the guardrail exists to prevent — and unknown/new models are precisely
   * the ones most likely to be mispriced. Buckets with a zero count never
   * trigger this (no charge is attributed, so a missing price is irrelevant).
   *
   * @throws {@link PricingMissingError} when a non-zero token bucket has no
   *   resolvable price.
   */
  usdMicrosForCall(args: {
    provider: string;
    model: string;
    inputTokens?: number;
    outputTokens?: number;
    cachedInputTokens?: number;
  }): number {
    let usd = 0;
    const charge = (kind: string, count: number): void => {
      if (count <= 0) return;
      // Preserve the existing default-kind fallback; only the FINAL `?? 0`
      // coercion is removed — a still-missing price now fails loud.
      const p =
        this.pricePerMillion(args.provider, args.model, kind) ??
        this.pricePerMillion(args.provider, args.model, this.defaultKind);
      if (p === null) {
        throw new PricingMissingError({
          provider: args.provider,
          model: args.model,
          tokenKind: kind,
        });
      }
      usd += (count * p) / 1_000_000;
    };
    charge("input", args.inputTokens ?? 0);
    charge("output", args.outputTokens ?? 0);
    charge("cached_input", args.cachedInputTokens ?? 0);
    if (usd <= 0) return 0;
    return Math.max(1, Math.ceil(usd * USD_MICROS_PER_USD));
  }
}
