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
 * SLICE 2 surface — additional fields land in SLICE 3 when `reserve` /
 * `commitEstimated` are wired. The shape is intentionally a superset: every
 * SLICE 3 addition will be backward-compatible (new optional fields only).
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
}
