import { BudgetClaim, UnitRef, PricingFreeze, ClaimEstimate, ApprovalRequired, DecisionOutcome, IdempotencyCache, SpendGuardClient } from '@spendguard/sdk';
export { ApprovalRequired, DecisionDenied, DecisionSkipped, DecisionStopped, SidecarUnavailable, SpendGuardError } from '@spendguard/sdk';

/**
 * Inputs handed to a {@link ClaimEstimator}. Provider-agnostic — every field
 * comes either from the Inngest runtime context (`stepId` / `runId` /
 * `attempt` / `inngestIdempotencyKey` / `eventId`) or from the wrapped
 * `step.ai` call site (`model` / `body`).
 *
 * The adapter treats this object as immutable (review-standards §14.5) — the
 * estimator MUST NOT mutate it.
 */
interface ClaimEstimatorInput {
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
type ClaimEstimator = (input: ClaimEstimatorInput) => readonly BudgetClaim[];
/** Optional content-signature derivation used by callers who want deterministic decisionIds. */
type CallSignatureFn = (input: ClaimEstimatorInput) => string;
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
interface WrapWithSpendGuardOptions {
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
    onApprovalRequired?: (err: ApprovalRequired, input: ClaimEstimatorInput) => Promise<DecisionOutcome | null | undefined>;
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
}

/**
 * @internal — slice of `@inngest/agent-kit`'s `step.ai` shape the adapter
 * depends on. The `runtimeCtx` parameter is intentionally typed as an
 * `InngestRuntimeCtx`-shaped optional so adapter callers can pass the real
 * `({ step })` destructured context through verbatim. The original
 * `@inngest/agent-kit@^0.13` signature is structurally a superset of this
 * shape — additional fields flow through untouched.
 */
interface StepAi {
    infer<TOut = unknown>(name: string, opts: {
        model: unknown;
        body: unknown;
    }, runtimeCtx?: InngestRuntimeCtx): Promise<TOut>;
    wrap<TFn extends (...args: never[]) => Promise<unknown>>(name: string, fn: TFn, ...args: Parameters<TFn>): Promise<Awaited<ReturnType<TFn>>>;
}
/**
 * @internal — slice of `@inngest/agent-kit`'s runtime-ctx shape the adapter
 * depends on. Documented in `@inngest/agent-kit@^0.13`'s `step.ai.infer`
 * signature.
 */
interface InngestRuntimeCtx {
    runId: string;
    eventId?: string;
    step: {
        id: string;
        attempt?: number;
        idempotencyKey?: string;
    };
}
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
declare function wrapWithSpendGuard(stepAi: StepAi, client: SpendGuardClient, options: WrapWithSpendGuardOptions): StepAi;

/**
 * Output of {@link deriveIdentity}. All four fields are deterministic
 * functions of `(tenantId, sessionId, stepId, inngestIdempotencyKey, runId)`.
 */
interface DerivedIdentity {
    /** UUIDv4-shaped, scope-namespaced under `"decision_id"`. */
    decisionId: string;
    /** `sg-` + 32 hex chars (BLAKE2b-128). Cross-language byte-identical. */
    idempotencyKey: string;
    /** Equal to `input.stepId`. */
    llmCallId: string;
    /** Equal to `input.stepId`. */
    stepId: string;
}
/**
 * Derive the SpendGuard identity tuple for an Inngest step boundary.
 *
 * Retry-safety contract (design.md §6 + review-standards §4):
 *
 *   - **Attempt-invariance:** Same `(tenantId, stepId, inngestIdempotencyKey,
 *     runId)` → same `idempotencyKey` regardless of `input.attempt`.
 *     Verified by R-02 (`tests/wrap.test.ts`).
 *   - **Run-scope:** A NEW Inngest function invocation (new `runId`) for
 *     the same step name produces a DIFFERENT `idempotencyKey` so a fresh
 *     run is NOT deduped against a prior run. Verified by R-08 / I-05.
 *   - **Seed precedence:** `inngestIdempotencyKey` wins over `stepId`
 *     when both are present, falls back to `stepId` when the consumer
 *     omits an explicit `step.ai`-level idempotency key. Verified by
 *     I-03 / I-04 / R-05.
 *
 * @param args.tenantId            - SpendGuard tenant the run is billed to.
 *                                    Forwarded to the canonical tuple's first
 *                                    slot.
 * @param args.input               - The {@link ClaimEstimatorInput} the
 *                                    factory built from the Inngest runtime
 *                                    context. `attempt` / `model` / `body` /
 *                                    `eventId` are deliberately NOT consumed
 *                                    here — they live on the estimator's
 *                                    inputs only.
 * @returns                          The four-field identity tuple. All four
 *                                    fields are stable across retries when
 *                                    the seed is stable.
 */
declare function deriveIdentity(args: {
    tenantId: string;
    input: ClaimEstimatorInput;
}): DerivedIdentity;
/**
 * Convenience: derive only the idempotencyKey component, useful when callers
 * want to probe the dedup contract without constructing a full identity. Same
 * canonical tuple as {@link deriveIdentity}.
 */
declare function deriveStepIdempotencyKey(args: {
    tenantId: string;
    runId: string;
    stepId: string;
    inngestIdempotencyKey?: string;
}): string;

/**
 * Pull a canonical `total_tokens` count out of an opaque provider result.
 *
 * Returns 0 when no recognisable usage payload is present (review-standards
 * §7.4). NEVER throws. Tolerates non-object `usage` fields (review-standards
 * §7.6 — drift tolerance).
 */
declare function extractTotalTokens(result: unknown): number;
/**
 * Pull a provider event id (commonly the chat-completion id) out of a
 * `step.ai` result.
 *
 * Probe order:
 *   1. `result.id`
 *   2. `result.response_metadata.id` / `result.responseMetadata.id`
 *   → `""`
 *
 * NEVER throws. Returns `""` for any unrecognised shape so the commit path
 * stays wire-safe.
 */
declare function extractProviderEventId(result: unknown): string;

declare const VERSION: "0.1.0-pre";

export { type CallSignatureFn, type ClaimEstimator, type ClaimEstimatorInput, type DerivedIdentity, type InngestRuntimeCtx, type StepAi, VERSION, type WrapWithSpendGuardOptions, deriveIdentity, deriveStepIdempotencyKey, extractProviderEventId, extractTotalTokens, wrapWithSpendGuard };
