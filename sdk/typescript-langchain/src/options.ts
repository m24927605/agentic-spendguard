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

import type { SpendGuardClient } from "@spendguard/sdk";

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
}
