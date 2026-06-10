// src/options.ts — LOCKED option shape (all camelCase).
//
// Copied verbatim from design.md §5 (verbatim contract — any drift is a P0
// finding, review-standards §2/§3) + the dated §6.7 amendment #3 (2026-06-11,
// `pricing`). NO fail-open field exists on this type (no `failOpen`, no
// `degradeOnUnavailable`, no `enforcementMode`) — adding one is a P0 finding
// (design §5 surface rules).

import type { BudgetClaim, PricingFreeze, SpendGuardClient } from "@spendguard/sdk";

export interface ClaimEstimatorInput {
  /** Deterministic flattened text of the step's messages (text parts only,
   *  joined with "\n" — same flatten discipline as D06 `flattenPromptText`). */
  stepText: string;
  /** Resolved run id for this step (derivation rule: design.md §6.3). */
  runId: string;
  /** Derived per-step call id (design.md §6.3). */
  llmCallId: string;
}

export type ClaimEstimator = (input: ClaimEstimatorInput) => readonly BudgetClaim[];

export interface SpendGuardProcessorOptions {
  /** Configured SpendGuardClient from @spendguard/sdk. Consumer owns the
   *  lifecycle (connect/handshake/close); the processor never closes it. */
  client: SpendGuardClient;
  /** Tenant the step bills to. REQUIRED and explicit (D06 discipline). */
  tenantId: string;
  /** Budget scope UUID for the projected claim's scopeId. Default: tenantId. */
  budgetId?: string;
  /** Ledger unit-row UUID — threads to BudgetClaim.unit.unitId on the wire.
   *  DAY-1 field (HARDEN_D05_UR). Ledger-backed reserves MUST set it;
   *  typical source is the SPENDGUARD_UNIT_ID env var at construction. */
  unitId?: string;
  /** Route label on ReserveRequest.route. Default "mastra-llm". */
  route?: string;
  /** Cap (atomic micros, bigint) used by the default claim projection when
   *  no claimEstimator is given. Mirrors D04's defaultBudgetMicrosCap. */
  defaultBudgetMicrosCap?: bigint;
  /** Custom pre-call claim projection. Default: chars/4 heuristic (§6.4). */
  claimEstimator?: ClaimEstimator;
  /** Override the run-id resolution (§6.3). Wins over Mastra-context-derived
   *  and content-derived run ids. */
  runIdProvider?: () => string;
  /**
   * Pricing freeze tuple the commit path repeats back to the ledger.
   * Must match the reservation's freeze: the production sidecar stamps
   * reservations with the LOADED BUNDLE's pricing freeze, so ledger-backed
   * commits that send the empty tuple are rejected with
   * `pricing freeze mismatch` (proved live by the COV_D38_05 demo). The
   * demos source it from `SPENDGUARD_PRICING_VERSION` +
   * `SPENDGUARD_PRICE_SNAPSHOT_HASH_HEX` + `SPENDGUARD_FX_RATE_VERSION` +
   * `SPENDGUARD_UNIT_CONVERSION_VERSION` (same convention as
   * `sdk/typescript-langchain`'s `pricing` option — D04 parity). Omitting
   * sends the empty tuple — fine when the reservation also carries the
   * empty tuple (recipe-style/no-bundle sidecars), rejected otherwise.
   * Additive optional field per the design.md §6.7 dated amendment #3
   * (2026-06-11, orchestrator-ratified).
   */
  pricing?: PricingFreeze;
}
