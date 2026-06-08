// `SpendGuardAgentsOptions` — the public, LOCKED option shape for the
// OpenAI Agents TS adapter.
//
// SLICE 2 ships only the minimum surface the factory + `bracketedGetResponse`
// reserve / commit wiring need: a substrate client, the tenant id the call
// is billed to, and an optional budget scope override. The richer
// field-for-field mirror of `design.md` §4 (`windowInstanceId`, `unit`,
// `pricing`, `claimEstimator`) is INTENTIONALLY deferred to SLICE 3+:
//
//   - SLICE 3 adds `claimEstimator` + cross-language fixture extension —
//     the default estimator derived from `inner.model` lands then so this
//     slice's tests stay focused on bracket discipline / surface lock.
//   - SLICE 4-5 demo wiring picks up `unit` / `pricing` once the demo's
//     real `commitEstimated()` shape lands.
//
// Every post-SLICE-2 addition is additive-optional so the SLICE 2 lock
// holds. Mirrors the D04 SLICE 2/3 + D06 SLICE 2/3 discipline exactly —
// minimal surface first, additive expansion at the slice that actually
// needs the field.
//
// All field names are camelCase per review-standards.md §3.

import type { SpendGuardClient } from "@spendguard/sdk";

/**
 * Locked options surface for {@link withSpendGuard} and
 * {@link SpendGuardAgentsModel}.
 *
 * SLICE 2 surface (LOCKED) — additional ADDITIVE OPTIONAL fields land in
 * SLICE 3+ when the cross-language fixture and the real-demo wiring need
 * them. Every post-SLICE-2 addition is backward-compatible (new optional
 * fields only) so consumers who pin against this minimal shape never break.
 *
 * @example
 * ```ts
 * import { withSpendGuard, runContext } from "@spendguard/openai-agents";
 * import { Agent, Runner } from "@openai/agents";
 * import { SpendGuardClient, newUuid7 } from "@spendguard/sdk";
 *
 * const client = new SpendGuardClient({ ... });
 * await client.connect();
 * await client.handshake();
 *
 * const guarded = withSpendGuard(innerModel, {
 *   client,
 *   tenantId: "tenant-prod",
 * });
 * const agent = new Agent({ name: "demo", model: guarded });
 *
 * const runId = newUuid7();
 * await runContext({ runId }, () => Runner.run(agent, "hello"));
 * ```
 */
export interface SpendGuardAgentsOptions {
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
   * Mirrors the D06 vercel-ai middleware's `tenantId` locking discipline —
   * cross-tenant misconfiguration is harder to silently mint when the field
   * is mandatory even though `SpendGuardClient` *does* expose a configured
   * `tenantId` of its own.
   */
  tenantId: string;

  /**
   * Optional budget scope override (UUID) used as the projected claim's
   * `scopeId`. When unset, SLICE 2 falls back to `tenantId` as the scopeId —
   * same default discipline as D04 SLICE 3 / D06 SLICE 3.
   *
   * Production consumers route to a team-specific budget by setting this
   * per adapter instance. The richer `windowInstanceId` / `unit` /
   * `pricing` fields the design.md §4 surface anticipates land in SLICE 4+;
   * see file-level JSDoc for the deferral rationale.
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
   * it through the bracket's reserve path).
   */
  unitId?: string;
}
