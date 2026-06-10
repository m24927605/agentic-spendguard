// `SpendGuardMiddlewareOptions` — the public, LOCKED option shape for the
// Vercel AI SDK adapter.
//
// SLICE 2/3 ships only the minimum surface the factory + `transformParams`
// reserve wiring need: a substrate client, the tenant id the call is billed
// to, and an optional budget scope override. The richer field-for-field
// mirror of `design.md` §4 (`windowInstanceId`, `unit`, `pricing`,
// `claimEstimator`, `callSignature`, `runIdProvider`, `route`,
// `providerEventIdExtractor`) is INTENTIONALLY deferred:
//
//   - SLICE 4 (`wrapGenerate`) needs `unit` + `pricing` to land the success
//     commit, so it picks up `unit` / `pricing` at that point.
//   - SLICE 5 (`wrapStream`) ditto for the streaming commit path.
//   - SLICE 6 (provider matrix) needs `claimEstimator` /
//     `providerEventIdExtractor` to ride the recorded fixtures, so they land
//     with the provider tests.
//
// Every post-SLICE-3 addition is additive-optional so the SLICE 2/3 lock
// holds. Mirrors the D04 SLICE 2/3 discipline exactly — minimal surface
// first, additive expansion at the slice that actually needs the field.
//
// All field names are camelCase per review-standards.md §1.6.

import type { PricingFreeze, SpendGuardClient } from "@spendguard/sdk";

/**
 * Constructor options for {@link createSpendGuardMiddleware}.
 *
 * SLICE 2/3 surface (LOCKED) — additional ADDITIVE OPTIONAL fields land in
 * SLICE 4+ when the commit / release paths are wired. Every post-SLICE-3
 * addition is backward-compatible (new optional fields only) so consumers
 * who pin against this minimal shape never break.
 *
 * @example
 * ```ts
 * import { createSpendGuardMiddleware } from "@spendguard/vercel-ai";
 * import { wrapLanguageModel } from "ai";
 * import { openai } from "@ai-sdk/openai";
 *
 * const middleware = createSpendGuardMiddleware({
 *   client,
 *   tenantId: "tenant-prod",
 * });
 * const model = wrapLanguageModel({
 *   model: openai("gpt-4o-mini"),
 *   middleware,
 * });
 * ```
 */
export interface SpendGuardMiddlewareOptions {
  /**
   * Configured `SpendGuardClient` instance from `@spendguard/sdk`. The
   * adapter does NOT own the client lifecycle — the consumer constructs it,
   * calls `connect()` / `handshake()`, and is responsible for `close()`.
   */
  client: SpendGuardClient;

  /**
   * Tenant id the call is billed to. Forwarded to the substrate as the
   * `reserve()` claim scope and as the first field of the idempotency-key
   * canonical tuple.
   *
   * Mirrors `pydantic_ai.py::SpendGuardModel.__init__`'s `tenant_id` arg —
   * the adapter does not infer a tenant from the client (the substrate
   * `SpendGuardClient` *does* expose `tenantId`, but D06's design.md §4
   * locks the middleware option as REQUIRED to keep the public surface
   * explicit; cross-tenant misconfiguration is harder to silently mint when
   * the field is mandatory).
   */
  tenantId: string;

  /**
   * Optional budget scope override (UUID) used as the projected claim's
   * `scopeId`. When unset, SLICE 3 falls back to `tenantId` as the scopeId
   * — same default discipline as D04 SLICE 3 / SLICE 5.
   *
   * Production consumers route to a team-specific budget by setting this
   * per middleware instance. The richer `windowInstanceId` / `unit` /
   * `pricing` fields the design.md §4 surface anticipates land in SLICE 4+;
   * see file-level JSDoc for the deferral rationale.
   */
  budgetId?: string;

  /**
   * Canonical-truth UUID of the ledger unit row. When set, threads to
   * `BudgetClaim.unit.unitId` on the wire so the sidecar ledger can
   * resolve the budget claim. Most operators source this from the
   * `SPENDGUARD_UNIT_ID` env var at middleware construction time.
   *
   * Omitting leaves the wire field empty and the ledger will reject the
   * reserve with `INVALID_REQUEST: claim[N].unit.unit_id empty` —
   * recipe-style integrations (no ledger reserve) MAY omit. NB: this is
   * the ledger UUID, distinct from the free-form unit slug — they are
   * NOT interchangeable.
   *
   * Additive optional field shipped under HARDEN_D05_UR (the SDK-side
   * `UnitRef.unitId` broadening landed in SLICE 1; this option threads
   * it through the middleware's `transformParams` reserve path).
   */
  unitId?: string;

  /**
   * Canonical-truth UUID of the ledger window-instance row. When set,
   * threads to `BudgetClaim.window_instance_id` on the wire. Most
   * operators source this from the `SPENDGUARD_WINDOW_INSTANCE_ID` env
   * var at middleware construction time.
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
