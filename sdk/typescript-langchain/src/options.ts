// `SpendGuardCallbackHandlerOptions` — the public, LOCKED option shape for the
// LangChain.js adapter.
//
// SLICE 2 ships only the minimum surface the skeleton needs: a substrate
// client, an optional tenant override, and an optional default budget cap.
// The full field-for-field mirror of the Python `SpendGuardChatModel`
// constructor (see `docs/specs/coverage/D04_langchain_ts/design.md` §4 and
// `implementation.md` §3.1 — `budgetId`, `windowInstanceId`, `unit`,
// `pricing`, `claimEstimator`, `route`, `callSignatureFn`,
// `claimEstimate`, `onApprovalRequired`) is INTENTIONALLY deferred to
// SLICE 3 so the throw-stubs in `handler.ts` stay self-contained.
//
// All field names are camelCase per review-standards.md §1.6.

import type { PricingFreeze, SpendGuardClient } from "@spendguard/sdk";

/**
 * Constructor options for {@link SpendGuardCallbackHandler}.
 *
 * SLICE 2 surface (LOCKED) — additional ADDITIVE OPTIONAL fields land in
 * SLICE 3+ when `reserve` / `commitEstimated` are wired. Every post-SLICE-2
 * addition is backward-compatible (new optional fields only) so the
 * SLICE 2 type lock holds.
 *
 * SLICE 5 deviation #1 (scope-routing only): added optional `budgetId` so
 * demo + production consumers can pin the projected claim's `scopeId` to a
 * specific budget UUID without subclassing the handler. The fuller
 * `unitId` / `windowInstanceId` / `pricing` / `claimEstimator` surface
 * design.md §4 anticipates remains deferred — the TS SDK substrate's
 * public `UnitRef` does not currently expose `unit_id` (`sdk/typescript/
 * src/client.ts::mapUnitRef` hardcodes empty), so a unit override would
 * be dead code today. The next D04 hardening slice picks up the
 * SDK-side broadening + adapter wire-through together.
 */
export interface SpendGuardCallbackHandlerOptions {
  /**
   * Configured `SpendGuardClient` instance from `@spendguard/sdk`. The
   * adapter does NOT own the client lifecycle — the consumer constructs it,
   * calls `connect()` / `handshake()`, and is responsible for `close()`.
   */
  client: SpendGuardClient;

  /**
   * Optional tenant override forwarded to the substrate when set. Defaults
   * to whatever tenant the `client` was configured with at construction
   * time. SLICE 3 surfaces it on the `reserve` path.
   */
  tenantId?: string;

  /**
   * Optional default budget cap in atomic micros (USD micros if `unit` is
   * `USD_MICROS`). Used by SLICE 3's fallback `claimEstimator` when the
   * consumer does not provide a custom estimator. `bigint` to avoid the
   * Number.MAX_SAFE_INTEGER cliff at $9.007e9.
   */
  defaultBudgetMicrosCap?: bigint;

  /**
   * Optional budget ID (UUID) used as the projected claim's `scopeId`.
   * When unset, the handler falls back to `tenantId` as the scopeId
   * (SLICE 3 default). Production consumers route to the right
   * team-budget by setting this per handler instance.
   *
   * Additive optional field, SLICE 5 deviation #1 (scope-routing only;
   * see interface JSDoc above for the deferred `unitId` /
   * `windowInstanceId` / pricing surface scope).
   */
  budgetId?: string;

  /**
   * Canonical-truth UUID of the ledger unit row. When set, threads to
   * `BudgetClaim.unit.unitId` on the wire so the sidecar ledger can
   * resolve the budget claim. Most operators source this from the
   * `SPENDGUARD_UNIT_ID` env var at adapter construction time.
   *
   * Omitting leaves the wire field empty and the ledger will reject the
   * reserve with `INVALID_REQUEST: claim[N].unit.unit_id empty` —
   * recipe-style integrations (no ledger reserve) MAY omit. NB: this is
   * the ledger UUID, distinct from the free-form unit slug — they are
   * NOT interchangeable.
   *
   * Additive optional field shipped under HARDEN_D05_UR (the SDK-side
   * `UnitRef.unitId` broadening landed in SLICE 1; this option threads
   * it through the adapter's reserve path).
   */
  unitId?: string;

  /**
   * Canonical-truth UUID of the ledger window-instance row. When set,
   * threads to `BudgetClaim.window_instance_id` on the wire. Most
   * operators source this from the `SPENDGUARD_WINDOW_INSTANCE_ID` env
   * var at handler construction time.
   *
   * Omitting leaves the wire field empty and the ledger will reject the
   * reserve with `INVALID_REQUEST: claim[N].window_instance_id empty` —
   * recipe-style integrations (no ledger reserve) MAY omit.
   *
   * Additive optional field shipped under HARDEN_D05_WI (mirror of the
   * HARDEN_D05_UR `unitId` broadening).
   */
  windowInstanceId?: string;

  /**
   * Demo/test-only escape hatch: when set (string-form integer), the
   * projected claim's `amountAtomic` uses this value INSTEAD of the
   * chars/4 heuristic. Mirrors the Python litellm callback's
   * `spendguard_estimate_override` convention so demo DENY steps can
   * blow past a seeded hard-cap deterministically. Production
   * consumers MUST NOT set this — pricing-table estimation is the
   * supported path.
   */
  estimateOverrideAtomic?: string;

  /**
   * Pricing freeze tuple the commit path repeats back to the ledger.
   * Must match the reservation's freeze (the demo sources it from
   * `SPENDGUARD_PRICING_VERSION` + `SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX`
   * + `SPENDGUARD_FX_RATE_VERSION` + `SPENDGUARD_UNIT_CONVERSION_VERSION`,
   * the same convention as the Python demos). Omitting sends the empty
   * tuple — fine when the ledger's reservation also carries the empty
   * tuple, rejected otherwise. Shipped under HARDEN_D05_WI.
   */
  pricing?: PricingFreeze;
}
