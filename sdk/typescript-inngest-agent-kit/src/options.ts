// `WrapWithSpendGuardOptions` — the public, LOCKED option shape for the
// `wrapWithSpendGuard` factory.
//
// D29 mirrors `SpendGuardCallbackHandlerOptions` (D04 §4) field-for-field
// minus `route` (defaults to `"llm.call.inngest"`). The richer field set
// (windowInstanceId / pricing / claimEstimate / onApprovalRequired) is
// surfaced here at SLICE 2+3 so the reserve / commit wiring is end-to-end
// testable against the locked design contract.
//
// All field names are camelCase per review-standards.md §1.5.
//
// Spec refs:
//   - design.md §4 (LOCKED public surface).
//   - implementation.md §3.1 (`src/options.ts` skeleton).
//   - review-standards.md §1.1, §1.4 (export gate + field-for-field mirror).

import type {
  ApprovalRequired,
  BudgetClaim,
  ClaimEstimate,
  DecisionOutcome,
  IdempotencyCache,
  PricingFreeze,
  SpendGuardClient,
  UnitRef,
} from "@spendguard/sdk";

/**
 * Inputs handed to a {@link ClaimEstimator}. Provider-agnostic — every field
 * comes either from the Inngest runtime context (`stepId` / `runId` /
 * `attempt` / `inngestIdempotencyKey` / `eventId`) or from the wrapped
 * `step.ai` call site (`model` / `body`).
 *
 * The adapter treats this object as immutable (review-standards §14.5) — the
 * estimator MUST NOT mutate it.
 */
export interface ClaimEstimatorInput {
  /** Inngest `step.id` — used as both `stepId` and `llmCallId`. */
  stepId: string;
  /** Inngest attempt counter (0 = first try, 1+ = retries). */
  attempt: number;
  /** Inngest's per-step idempotency key when the `step.ai` call supplied one. */
  inngestIdempotencyKey?: string;
  /** Inngest function `runId`. */
  runId: string;
  /** Inngest event id when available. */
  eventId?: string;
  /** Wrapped `step.ai` model handle — provider-agnostic. */
  model: unknown;
  /** Wrapped `step.ai` body payload — provider-agnostic. */
  body: unknown;
}

/** Maps a {@link ClaimEstimatorInput} onto the `projectedClaims` array. */
export type ClaimEstimator = (input: ClaimEstimatorInput) => readonly BudgetClaim[];

/** Optional content-signature derivation used by callers who want deterministic decisionIds. */
export type CallSignatureFn = (input: ClaimEstimatorInput) => string;

/**
 * Locked options surface for {@link wrapWithSpendGuard}.
 *
 * Field-for-field mirror of design.md §4 (and of
 * `SpendGuardCallbackHandlerOptions` from D04) minus `route` (defaults to
 * `"llm.call.inngest"`). Additive-only after SLICE 3 — every post-SLICE-3
 * addition is backward-compatible (new optional fields only).
 *
 * @example
 * ```ts
 * import { wrapWithSpendGuard } from "@spendguard/inngest-agent-kit";
 * import { SpendGuardClient } from "@spendguard/sdk";
 *
 * const client = new SpendGuardClient({ ... });
 * await client.connect();
 * await client.handshake();
 *
 * inngest.createFunction({ id: "agent-fn" }, { event: "agent/run" },
 *   async ({ step }) => {
 *     const sgStep = wrapWithSpendGuard(step.ai, client, {
 *       tenantId: "tenant-prod",
 *       budgetId: BUDGET_ID,
 *       windowInstanceId: WINDOW_ID,
 *       unit: { unit: "USD_MICROS", denomination: 1 },
 *       pricing: { pricingVersion: PRICING_VERSION, pricingHash: new Uint8Array(0) },
 *       claimEstimator: () => [{
 *         scopeId: BUDGET_ID,
 *         amountAtomic: "1000000",
 *         unit: { unit: "USD_MICROS", denomination: 1 },
 *       }],
 *     });
 *     return await sgStep.infer("call-openai", { model, body });
 *   });
 * ```
 */
export interface WrapWithSpendGuardOptions {
  /**
   * Tenant id the call is billed to. Mirrors the D08 `withSpendGuard` /
   * D06 `vercel-ai` middleware tenant-locking discipline — cross-tenant
   * misconfiguration is harder to silently mint when the field is mandatory
   * even though `SpendGuardClient` *does* expose a configured `tenantId`
   * of its own.
   */
  tenantId: string;

  /**
   * Optional budget id (UUID) used as the projected claim's default
   * `scopeId` when the consumer's {@link ClaimEstimator} returns claims
   * without their own scope. When unset, the adapter falls back to
   * `tenantId` as the scopeId. Production consumers route to a
   * team-specific budget by setting this per `wrapWithSpendGuard` call.
   */
  budgetId?: string;

  /**
   * Optional budget window id (UUID). Forwarded to the substrate when set.
   * Mirrors D04 §4 / D08 §4 — same shape, same forwarding semantics.
   */
  windowInstanceId?: string;

  /**
   * Optional canonical money unit. Defaults to `{ unit: "USD_MICROS",
   * denomination: 1 }` on the commit path when unset.
   */
  unit?: UnitRef;

  /**
   * Optional pricing freeze. Empty-freeze default is honored on the commit
   * path when unset — the sidecar's server-side defaults take over.
   */
  pricing?: PricingFreeze;

  /**
   * Project the pre-call `BudgetClaim[]` from a {@link ClaimEstimatorInput}.
   * Called exactly once per `infer` / `wrap` invocation. The default — when
   * the consumer does not supply one — is a single zero-amount probe claim
   * scoped to `budgetId ?? tenantId`; production consumers MUST override.
   */
  claimEstimator?: ClaimEstimator;

  /**
   * Optional route override. Defaults to `"llm.call.inngest"` —
   * design.md §4 LOCKED.
   */
  route?: string;

  /**
   * Optional content-signature override. When supplied, the adapter feeds
   * the signature through `deriveUuidFromSignature` for `decisionId` /
   * `llmCallId` — same as D08. Default: the step identity itself drives
   * the identity derivation (see `src/ids.ts`).
   */
  callSignatureFn?: CallSignatureFn;

  /**
   * Optional fine-grained claim estimate forwarded verbatim on the reserve
   * request. Mirrors design.md §4 — `claimEstimator` projects the bulk
   * claim shape; `claimEstimate` carries higher-fidelity numeric hints.
   */
  claimEstimate?: ClaimEstimate;

  /**
   * Optional approval-resume callback. Called when reserve throws
   * `ApprovalRequired`; a non-nullish return value resumes the call with
   * the supplied outcome. A `null` / `undefined` return value re-throws
   * the original error. Mirrors D04 / D06 / D08 review-standards §5.4-5.5.
   */
  onApprovalRequired?: (
    err: ApprovalRequired,
    input: ClaimEstimatorInput,
  ) => Promise<DecisionOutcome | null | undefined>;

  /**
   * Optional same-process idempotency cache. When supplied, identical
   * `idempotencyKey`s short-circuit the sidecar `reserve` round-trip.
   * Inngest retries with the SAME `step.id` derive byte-identical keys
   * (see `src/ids.ts`), so the cache returns the cached outcome and the
   * adapter records ONE PRE / ONE POST across N retries — the
   * retry-dedup contract (review-standards §4).
   *
   * When unset, the layered-defence path applies: the sidecar's own
   * idempotency dedup catches the duplicate `idempotencyKey` and the
   * cache still returns one logical PRE per step (proven by R-06).
   */
  idempotencyCache?: IdempotencyCache;

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
   * it through the adapter's default `projectClaims` reserve path).
   * Note: when the consumer supplies a custom `claimEstimator`, that
   * function is responsible for setting `unit.unitId` on its returned
   * claims — `claimEstimator` always wins (Python parity).
   */
  unitId?: string;

  /**
   * Demo/test-only escape hatch: when set (string-form integer), every
   * projected claim's `amountAtomic` uses this value INSTEAD of the
   * default / `claimEstimator`-produced amount. Mirrors the Python
   * litellm callback's `spendguard_estimate_override` convention so demo
   * DENY steps can blow past a seeded hard-cap deterministically.
   * Production consumers MUST NOT set this — pricing-table estimation is
   * the supported path. Shipped under HARDEN_D05_WI.
   */
  estimateOverrideAtomic?: string;
}

/**
 * Type guard helper exported for the SLICE-3 factory validate path. Adapter-
 * internal but exported so tests can probe individual field validations
 * without importing the factory.
 */
export function isObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object";
}

/**
 * @internal — type-level marker. We re-export the `SpendGuardClient` type
 * alias here so adapter consumers do not need a second `@spendguard/sdk`
 * import when they only need to type the options bag. The runtime symbol
 * is NOT re-exported (review-standards §1.7) — only the type alias.
 */
export type { SpendGuardClient };
