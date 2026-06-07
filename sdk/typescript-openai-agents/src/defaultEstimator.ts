// SLICE 3 — default `claimEstimator` derived from `inner.model`.
//
// design.md §7 locked decision #5 says the table is byte-identical to
// the Python `_default_estimator.MODEL_BASELINE_TOKENS` table. The current
// Python implementation at `sdk/python/src/spendguard/integrations/
// _default_estimator.py` actually dispatches to a per-model tokenizer
// (Strategy A), not a fixed baseline table — there is no literal
// `MODEL_BASELINE_TOKENS` symbol in the Python tree today.
//
// **Deviation #1 vs design.md §7 decision #5** (documented inline; mirrors
// D04 SLICE 2 / D06 SLICE 3 surface-narrowing deviations):
//   The TS adapter ships the design.md §11 literal-numbers table as the
//   v0.1.x default. Per-model tokenizer dispatch is the Python sibling's
//   v0.5.x extension and lands in a future TS minor as additive optional
//   (a user-supplied `claimEstimator` already covers the escape hatch).
//   Cross-language fixture verification of the literal table values runs
//   in `tests/integration.test.ts` so a future port of the Python tokenizer
//   dispatch can add fresh fixture rows without invalidating the v0.1.x
//   baselines.
//
// SLICE 2 / `core.ts` keeps the safe `0`-amount projection. SLICE 3 wires
// the table on top of the SLICE 2 surface so `withSpendGuard(model)` with
// no `claimEstimator` arg ALREADY routes a sensible baseline through the
// reserve. A caller-supplied `claimEstimator` always wins (Python parity
// "explicit non-null wins", `_NO_DEFAULT` discipline).

import type { BudgetClaim, UnitRef } from "@spendguard/sdk";

/**
 * Byte-identical to the design.md §11 literal table — the SLICE 3
 * cross-language fixture extension (`openai_agents` section in
 * `sdk/fixtures/cross-language/v1.json`) reads these numbers and asserts
 * parity with the Python reference.
 *
 * Default fallback is `800` tokens for unknown model strings — design.md
 * §11 + reviewer gate 9.5.
 */
export const MODEL_BASELINE_TOKENS: Readonly<Record<string, number>> = Object.freeze({
  "gpt-4o-mini": 500,
  "gpt-4o": 1500,
  "gpt-4.1-mini": 500,
  "gpt-4.1": 1500,
  o1: 3000,
  "o3-mini": 1500,
  o3: 3000,
});

/** Unknown-model fallback baseline. design.md §11 + reviewer gate 9.5. */
export const DEFAULT_BASELINE_TOKENS = 800 as const;

/**
 * Resolve a baseline token count for an inner model name string. Returns
 * the table value when present, the `DEFAULT_BASELINE_TOKENS` fallback
 * otherwise. Case-sensitive — design.md §11 leaves casing to the inner
 * model id, which OpenAI ships lowercased.
 */
export function resolveBaselineTokens(modelName: string): number {
  return MODEL_BASELINE_TOKENS[modelName] ?? DEFAULT_BASELINE_TOKENS;
}

/**
 * Default `ClaimEstimator` shape returned by the SLICE 3 wiring. Mirrors
 * the design.md §4 `ClaimEstimator = (input: unknown) => BudgetClaim[]`
 * surface; the SLICE 2 LOCKED options surface does NOT yet expose this
 * type (it lands additive-optional in a future minor — see
 * `options.ts` JSDoc). Until then, the bracket invokes
 * `defaultClaimEstimator(...)` directly on the inner model name and the
 * caller's options.
 */
export type ClaimEstimator = (input: unknown) => BudgetClaim[];

/**
 * Build the default `ClaimEstimator` for an inner model + scope.
 *
 * @param opts.scopeId - The `BudgetClaim.scopeId` to thread on the claim —
 *   typically `opts.budgetId` falling back to `opts.tenantId` (the SLICE 2
 *   default discipline).
 * @param opts.unit - The `UnitRef` to thread on the claim. SLICE 3 uses the
 *   substrate-default micro-dollar unit; a future minor lifts the
 *   consumer-provided `UnitRef` onto the options surface (design.md §4).
 * @param opts.modelName - The inner model id (from `inner.model`). Drives
 *   the `MODEL_BASELINE_TOKENS` lookup.
 * @returns A `ClaimEstimator` that, when called, returns a one-element
 *   `BudgetClaim[]` with `amountAtomic` = baseline token count rendered as
 *   string (substrate wire shape is int64-as-string).
 */
export function defaultClaimEstimator(opts: {
  scopeId: string;
  unit: UnitRef;
  modelName: string;
}): ClaimEstimator {
  const baseline = resolveBaselineTokens(opts.modelName);
  const amountAtomic = String(baseline);
  return (_input: unknown): BudgetClaim[] => [
    {
      scopeId: opts.scopeId,
      amountAtomic,
      unit: opts.unit,
    },
  ];
}
