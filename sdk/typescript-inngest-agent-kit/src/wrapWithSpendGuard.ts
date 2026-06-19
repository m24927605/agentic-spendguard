// `wrapWithSpendGuard` — factory that returns a `step.ai`-shaped object whose
// `infer()` / `wrap()` calls thread through SpendGuard reserve → provider →
// commit.
//
// Headline contract (review-standards §4): Inngest retries of the SAME
// step.id derive a byte-identical `idempotencyKey`, so the D05 in-process
// `DecisionCache` (or the sidecar's own idempotency dedup) returns the
// cached outcome and the adapter records ONE `LLM_CALL_PRE` audit row
// across N attempts.
//
// LOCKED behaviour (design.md §5 + review-standards §3):
//
//   1. PRE — compute `(decisionId, idempotencyKey, llmCallId, stepId)` via
//      `deriveIdentity(...)`. The same triple `(tenantId, stepId,
//      inngestIdempotencyKey, runId)` across all retry attempts derives the
//      same key; `attempt` is NOT part of the seed.
//   2. PRE — optional in-process cache lookup: if the consumer supplied
//      `opts.idempotencyCache`, a HIT short-circuits the sidecar round-trip
//      and the cached `DecisionOutcome` flows through to the inner call.
//      MISS → fall through to `client.reserve(...)` with `trigger=LLM_CALL_PRE`.
//      DENY / STOP / SKIP / APPROVAL → typed error → inner NEVER reached
//      (review-standards §5.1-5.7).
//   3. PRE — `SidecarUnavailable` propagates by default (review-standards §5.2).
//   4. POST SUCCESS — fire `client.commitEstimated(...)` with
//      `outcome="SUCCESS"`, `estimatedAmountAtomic=String(extractTotalTokens(result))`,
//      `providerEventId=extractProviderEventId(result)`. Cache the outcome on
//      the way out so a later retry against the same key short-circuits.
//   5. POST PROVIDER_ERROR — provider throws → fire `client.commitEstimated(...)`
//      with `outcome="PROVIDER_ERROR"`, then re-throw. Commit failure is
//      logged but does NOT mask the original provider error (review-standards
//      §5.10).
//
// Anti-scope (do NOT add):
//   - Per-stream / per-chunk gating (LOCKED OUT — `step.ai.infer` is
//     non-streaming, design.md §3).
//   - Cross-step budget enforcement (contract-layer concern, design.md §3).
//   - Approval-resume UI (out of scope, design.md §3).
//   - Module-level mutable state (review-standards §11.2 — closure-only).

import {
  ApprovalRequired,
  type BudgetClaim,
  type CommitEstimatedRequest,
  type DecisionOutcome,
  type PricingFreeze,
  type ReserveRequest,
  type SpendGuardClient,
  type UnitRef,
} from "@spendguard/sdk";
import { extractProviderEventId, extractTotalTokens } from "./extract.js";
import { deriveIdentity } from "./ids.js";
import type { ClaimEstimatorInput, WrapWithSpendGuardOptions } from "./options.js";

// ── Defaults the LOCKED options surface deliberately omits ─────────────────
//
// Mirror D04 / D06 / D08 — pick sensible defaults for fields the LOCKED
// options surface does not require so the SLICE 3 wiring is end-to-end
// testable without expanding the public type.

/** Default route label surfaced on `ReserveRequest.route`. LOCKED — design.md §4. */
const DEFAULT_ROUTE = "llm.call.inngest";

/** Default budget unit — micro-dollars, the substrate's canonical money unit. */
const DEFAULT_UNIT: UnitRef = { unit: "USD_MICROS", denomination: 1 };

/**
 * Empty pricing freeze. Commits ride with a blank freeze when the consumer
 * does not supply one; the sidecar's server-side defaults take over.
 */
const EMPTY_PRICING: PricingFreeze = {
  pricingVersion: "",
  pricingHash: new Uint8Array(0),
};

/** Trigger constant; only LLM_CALL_PRE for SLICE 2/3's reserve. */
const TRIGGER_PRE = "LLM_CALL_PRE" as const;

// ── Inngest type aliases ───────────────────────────────────────────────────
//
// Narrow alias for the @inngest/agent-kit `step.ai` shape we depend on.
// We intentionally type-alias the slice instead of importing the public type
// so a minor 0.13.x churn does not break the build. The shape mirrors
// `@inngest/agent-kit@^0.13`'s `step.ai` namespace verbatim.

/**
 * @internal — slice of `@inngest/agent-kit`'s `step.ai` shape the adapter
 * depends on. The `runtimeCtx` parameter is intentionally typed as an
 * `InngestRuntimeCtx`-shaped optional so adapter callers can pass the real
 * `({ step })` destructured context through verbatim. The original
 * `@inngest/agent-kit@^0.13` signature is structurally a superset of this
 * shape — additional fields flow through untouched.
 */
export interface StepAi {
  infer<TOut = unknown>(
    name: string,
    opts: { model: unknown; body: unknown },
    runtimeCtx?: InngestRuntimeCtx,
  ): Promise<TOut>;
  wrap<TFn extends (...args: never[]) => Promise<unknown>>(
    name: string,
    fn: TFn,
    ...args: Parameters<TFn>
  ): Promise<Awaited<ReturnType<TFn>>>;
}

/**
 * @internal — slice of `@inngest/agent-kit`'s runtime-ctx shape the adapter
 * depends on. Documented in `@inngest/agent-kit@^0.13`'s `step.ai.infer`
 * signature.
 */
export interface InngestRuntimeCtx {
  runId: string;
  eventId?: string;
  step: { id: string; attempt?: number; idempotencyKey?: string };
}

// ── Per-step state (closure-scoped) ────────────────────────────────────────
//
// review-standards §11.2 — the wrap maintains NO module-level mutable
// state. Per-step PRE/POST correlation is a local variable within the
// `runReserveAndCommit(...)` async function; concurrent step.ai calls are
// isolated by the async-function frame.

/**
 * Wrap an Inngest `step.ai` namespace so every `infer()` / `wrap()` call
 * passes through SpendGuard reserve → provider → commit transparently.
 *
 * **Retry-safety** — the headline contract.
 *
 * The SpendGuard `idempotencyKey` is derived from Inngest's own step
 * identity, so a retried step short-circuits to the cached decision and
 * the adapter records ONE `LLM_CALL_PRE` audit row across N attempts. The
 * seed is `step.idempotencyKey ?? step.id` (both are attempt-invariant by
 * Inngest's own contract).
 *
 * When the consumer supplies `opts.idempotencyCache`, the in-process cache
 * absorbs the duplicate `reserve` without crossing the sidecar UDS. When
 * not, the sidecar's own idempotency dedup catches the duplicate
 * `idempotencyKey` — layered defence per review-standards §4.3 and §4.6.
 *
 * @param stepAi   - The `@inngest/agent-kit` `step.ai` namespace from the
 *                   Inngest function's `({ step })` destructured arg.
 * @param client   - Configured `SpendGuardClient` instance. The adapter does
 *                   NOT own the client lifecycle.
 * @param options  - {@link WrapWithSpendGuardOptions} — LOCKED surface.
 *
 * @returns        - A new `StepAi`-shaped object whose `infer` / `wrap`
 *                   signatures match the original. Type-preserving — the
 *                   wrapped `Promise<TOut>` flows through verbatim.
 *
 * @throws DecisionDenied (and subclasses — `DecisionStopped`,
 *   `ApprovalRequired` without `onApprovalRequired`, `DecisionSkipped`)
 *   — propagates so the Inngest step fails before the provider call fires.
 * @throws SidecarUnavailable — propagates as-is when the sidecar is
 *   unreachable. Strict-mode default (review-standards §5.2 / §5.7).
 *
 * @example
 * ```ts
 * inngest.createFunction({ id: "agent-fn" }, { event: "agent/run" },
 *   async ({ step }) => {
 *     const sgStep = wrapWithSpendGuard(step.ai, client, {
 *       tenantId,
 *       budgetId,
 *       claimEstimator: () => [{
 *         scopeId: budgetId, amountAtomic: "1000000",
 *         unit: { unit: "USD_MICROS", denomination: 1 },
 *       }],
 *     });
 *     return await sgStep.infer("call-openai", { model, body });
 *   });
 * ```
 */
export function wrapWithSpendGuard(
  stepAi: StepAi,
  client: SpendGuardClient,
  options: WrapWithSpendGuardOptions,
): StepAi {
  validateOptions(options);

  const route = options.route ?? DEFAULT_ROUTE;
  const unit = options.unit ?? DEFAULT_UNIT;
  const pricing = options.pricing ?? EMPTY_PRICING;
  const tenantId = options.tenantId;

  async function runReserveAndCommit<TOut>(
    body: () => Promise<TOut>,
    inputBuilder: () => ClaimEstimatorInput,
  ): Promise<TOut> {
    const input = inputBuilder();
    const id = deriveIdentity({ tenantId, input });
    // HARDEN_D05_WI — claims projected ONCE so the reserve request and the
    // commit path share the exact same UnitRef (the ledger rejects commits
    // whose `payload.unit_id` differs from the reservation's). On the
    // cache-hit path the reserve is skipped but the recomputed claims still
    // describe the reservation the cached outcome points at (the estimator
    // is deterministic per review-standards §3.6).
    const claims = projectClaims(input);
    const commitUnit = claims[0]?.unit ?? unit;

    // PRE — in-process cache probe (review-standards §4.3 / §4.6).
    let outcome: DecisionOutcome | undefined;
    if (options.idempotencyCache !== undefined) {
      const cached = options.idempotencyCache.get(id.idempotencyKey);
      if (cached !== undefined) {
        outcome = cached;
      }
    }

    // PRE — sidecar reserve (cache miss).
    if (outcome === undefined) {
      try {
        outcome = await client.reserve(buildReserveRequest(input, id, claims));
      } catch (err) {
        if (err instanceof ApprovalRequired && options.onApprovalRequired !== undefined) {
          const resumed = await options.onApprovalRequired(err, input);
          if (resumed === null || resumed === undefined) {
            throw err;
          }
          outcome = resumed;
        } else {
          // DecisionDenied / DecisionStopped / DecisionSkipped /
          // SidecarUnavailable / ApprovalRequired (no resumer) — propagate
          // (review-standards §5.1-§5.7). The Inngest step body throws,
          // which Inngest records as a failed step; no provider call leaves
          // the process.
          throw err;
        }
      }

      // Cache the freshly-minted outcome so retries against the same key
      // short-circuit (review-standards §4.3).
      if (options.idempotencyCache !== undefined && outcome !== undefined) {
        options.idempotencyCache.set(id.idempotencyKey, outcome);
      }
    }

    // POST — body + commit.
    try {
      const result = await body();
      const totalTokens = extractTotalTokens(result);
      const providerEventId = extractProviderEventId(result);
      // HARDEN_D05_WI — SUCCESS commit failure is warned, NOT thrown: the
      // provider result has already been produced and a commit-side fault
      // must not corrupt it (mirrors the vercel-ai / openai-agents
      // `safeCommit` convention; sidecar TTL reconciles any orphaned
      // reservation via the audit chain).
      try {
        await client.commitEstimated(
          buildCommitRequest(input, id, outcome, {
            outcomeStatus: "SUCCESS",
            estimatedAmountAtomic: String(totalTokens),
            providerEventId,
            // HARDEN_D05_WI — reuse the reserve-time unit so payload.unit_id
            // matches the reservation.
            unit: commitUnit,
            pricing,
          }),
        );
      } catch (commitErr) {
        const reason = commitErr instanceof Error ? commitErr.message : String(commitErr);
        console.warn(
          `[spendguard:inngest-agent-kit] SUCCESS commit failed for stepId=${id.stepId}; ` +
            `provider result preserved (${reason})`,
        );
      }
      return result;
    } catch (providerErr) {
      // Provider-side throw → emit a PROVIDER_ERROR commit, then re-throw.
      // Commit failure is logged but MUST NOT mask the original error
      // (review-standards §5.10).
      try {
        await client.commitEstimated(
          buildCommitRequest(input, id, outcome, {
            outcomeStatus: "PROVIDER_ERROR",
            estimatedAmountAtomic: "0",
            providerEventId: "",
            // HARDEN_D05_WI — reserve-time unit + freeze tuple must match
            // the reservation even on the PROVIDER_ERROR commit path.
            unit: commitUnit,
            pricing,
            errorMessage: providerErr instanceof Error ? providerErr.message : String(providerErr),
          }),
        );
      } catch (commitErr) {
        const reason = commitErr instanceof Error ? commitErr.message : String(commitErr);
        console.warn(
          `[spendguard:inngest-agent-kit] PROVIDER_ERROR commit failed for stepId=${id.stepId}; ` +
            `original provider error preserved (${reason})`,
        );
      }
      throw providerErr;
    }
  }

  function buildReserveRequest(
    input: ClaimEstimatorInput,
    id: ReturnType<typeof deriveIdentity>,
    claims: BudgetClaim[],
  ): ReserveRequest {
    const req: ReserveRequest = {
      trigger: TRIGGER_PRE,
      runId: input.runId,
      stepId: id.stepId,
      llmCallId: id.llmCallId,
      decisionId: id.decisionId,
      route,
      projectedClaims: claims,
      idempotencyKey: id.idempotencyKey,
    };
    if (options.claimEstimate !== undefined) {
      req.claimEstimate = options.claimEstimate;
    }
    if (options.windowInstanceId !== undefined) {
      // ReserveRequest does not LOCK a `windowInstanceId` field on the
      // public wire shape (D05 §4); the substrate threads it through the
      // claim's scope when needed. Adapter forwards verbatim to a future
      // optional field without taking a hard dep on it landing.
      (req as ReserveRequest & { windowInstanceId?: string }).windowInstanceId =
        options.windowInstanceId;
    }
    return req;
  }

  function buildCommitRequest(
    input: ClaimEstimatorInput,
    id: ReturnType<typeof deriveIdentity>,
    outcome: DecisionOutcome,
    extras: {
      outcomeStatus: CommitEstimatedRequest["outcome"];
      estimatedAmountAtomic: string;
      providerEventId: string;
      unit: UnitRef;
      pricing: PricingFreeze;
      errorMessage?: string;
    },
  ): CommitEstimatedRequest {
    const req: CommitEstimatedRequest = {
      runId: input.runId,
      stepId: id.stepId,
      llmCallId: id.llmCallId,
      decisionId: outcome.decisionId,
      reservationId: outcome.reservationIds[0] ?? "",
      estimatedAmountAtomic: extras.estimatedAmountAtomic,
      unit: extras.unit,
      pricing: extras.pricing,
      providerEventId: extras.providerEventId,
      outcome: extras.outcomeStatus,
    };
    if (extras.errorMessage !== undefined) {
      req.actualErrorMessage = extras.errorMessage;
    }
    return req;
  }

  function projectClaims(input: ClaimEstimatorInput): BudgetClaim[] {
    if (options.claimEstimator !== undefined) {
      // The estimator is called exactly once per `infer` / `wrap`
      // (review-standards §3.6); a throw here propagates as-is
      // (review-standards §5.6).
      return applyEstimateOverride([...options.claimEstimator(input)]);
    }
    // Default probe claim — zero amount, scoped to budgetId ?? tenantId.
    // Production consumers MUST override; the default keeps the SLICE 3
    // wiring end-to-end testable without forcing every consumer to ship a
    // custom estimator.
    //
    // HARDEN_D05_UR — thread caller-supplied unitId onto the default-claim
    // wire UnitRef. Omitted unitId keeps the pre-HARDEN_D05_UR wire shape
    // (substrate `mapUnitRef` coerces to "").
    const claimUnit: UnitRef = options.unitId ? { ...unit, unitId: options.unitId } : unit;
    return applyEstimateOverride([
      {
        scopeId: options.budgetId ?? tenantId,
        amountAtomic: "0",
        unit: claimUnit,
        // HARDEN_D05_WI — thread caller-supplied windowInstanceId onto the
        // wire claim (substrate coerces omitted to "").
        ...(options.windowInstanceId ? { windowInstanceId: options.windowInstanceId } : {}),
      },
    ]);
  }

  /**
   * Demo/test-only: `estimateOverrideAtomic` replaces every claim's
   * `amountAtomic` (mirrors the Python litellm callback's
   * spendguard_estimate_override). No-op when the option is unset or not
   * a string-form integer. Shipped under HARDEN_D05_WI.
   */
  function applyEstimateOverride(claims: BudgetClaim[]): BudgetClaim[] {
    const override = options.estimateOverrideAtomic;
    if (override !== undefined && /^[0-9]+$/.test(override)) {
      return claims.map((claim) => ({ ...claim, amountAtomic: override }));
    }
    return claims;
  }

  function inputFromCtx(
    ctx: InngestRuntimeCtx | undefined,
    name: string,
    model: unknown,
    body: unknown,
  ): ClaimEstimatorInput {
    // When `runtimeCtx` is undefined (test harness path), degrade gracefully:
    // use `name` as `stepId` and empty string `runId` (review-standards §2.4).
    const stepId = ctx?.step.id ?? name;
    const input: ClaimEstimatorInput = {
      stepId,
      attempt: ctx?.step.attempt ?? 0,
      runId: ctx?.runId ?? "",
      model,
      body,
    };
    if (ctx?.step.idempotencyKey !== undefined) {
      input.inngestIdempotencyKey = ctx.step.idempotencyKey;
    }
    if (ctx?.eventId !== undefined) {
      input.eventId = ctx.eventId;
    }
    return input;
  }

  return {
    async infer(name, opts, runtimeCtx) {
      const ctx = runtimeCtx as InngestRuntimeCtx | undefined;
      return runReserveAndCommit(
        () => stepAi.infer(name, opts, runtimeCtx),
        () => inputFromCtx(ctx, name, opts.model, opts.body),
      );
    },
    wrap<TFn extends (...args: never[]) => Promise<unknown>>(
      name: string,
      fn: TFn,
      ...args: Parameters<TFn>
    ): Promise<Awaited<ReturnType<TFn>>> {
      // `step.ai.wrap` does not have a documented runtime-ctx slot in
      // `@inngest/agent-kit@^0.13`; the adapter degrades to the
      // `name`-as-stepId / empty `runId` path (review-standards §2.4).
      // When a future minor exposes ctx, the adapter picks it up via the
      // last positional argument convention.
      const maybeCtx = (args[args.length - 1] ?? undefined) as InngestRuntimeCtx | undefined;
      const ctx =
        maybeCtx !== undefined && typeof maybeCtx === "object" && "step" in maybeCtx
          ? maybeCtx
          : undefined;
      return runReserveAndCommit(
        () => stepAi.wrap(name, fn, ...args),
        () => inputFromCtx(ctx, name, undefined, args),
      ) as Promise<Awaited<ReturnType<TFn>>>;
    },
  };
}

// ── Internal helpers ───────────────────────────────────────────────────────

function validateOptions(opts: WrapWithSpendGuardOptions): void {
  if (opts === null || typeof opts !== "object") {
    throw new TypeError("wrapWithSpendGuard: opts must be an object");
  }
  if (typeof opts.tenantId !== "string" || opts.tenantId.length === 0) {
    throw new TypeError("wrapWithSpendGuard: opts.tenantId is required (non-empty string)");
  }
}
