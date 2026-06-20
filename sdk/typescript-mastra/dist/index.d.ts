import { MessageList } from '@mastra/core/agent';
import { Processor, ProcessInputStepArgs, ProcessLLMResponseArgs, ProcessOutputStepArgs, ProcessAPIErrorArgs, ProcessLLMRequestArgs } from '@mastra/core/processors';
import { BudgetClaim, SpendGuardClient, PricingFreeze } from '@spendguard/sdk';
export { DecisionDenied, SidecarUnavailable, SpendGuardError } from '@spendguard/sdk';

interface ClaimEstimatorInput {
    /** Deterministic flattened text of the step's messages (text parts only,
     *  joined with "\n" — same flatten discipline as D06 `flattenPromptText`). */
    stepText: string;
    /** Resolved run id for this step (derivation rule: design.md §6.3). */
    runId: string;
    /** Derived per-step call id (design.md §6.3). */
    llmCallId: string;
}
type ClaimEstimator = (input: ClaimEstimatorInput) => readonly BudgetClaim[];
interface SpendGuardProcessorOptions {
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

declare class SpendGuardProcessor implements Processor {
    /** Required by the installed `Processor` interface (V1 pin above). */
    readonly id = "spendguard-processor";
    /** Stable processor name (Mastra requires one per processor instance). */
    readonly name = "spendguard-processor";
    private readonly opts;
    private readonly inflight;
    constructor(options: SpendGuardProcessorOptions);
    /**
     * RESERVE — before-LLM-step boundary (design §6.1 row 1). Fires at every
     * step including tool-call continuations.
     */
    processInputStep(args: ProcessInputStepArgs): Promise<undefined>;
    /**
     * SUCCESS COMMIT — after each provider response (design §6.1 row 3).
     * Usage actuals when the finish chunk exposes them (V4 pin in usage.ts);
     * §6.6 LOCKED estimated-amount fallback otherwise.
     *
     * Commit-path errors are SWALLOWED (logged at error level) — design §7.4
     * LOCKED pre/post asymmetry: a post-call commit failure cannot un-spend;
     * the sidecar TTL sweep settles the reservation. This swallow must never
     * creep into the pre-dispatch reserve path (review-standards §2.6).
     */
    processLLMResponse(args: ProcessLLMResponseArgs): Promise<undefined>;
    /**
     * Backstop COMMIT — after the step's output is assembled (design §6.1 row
     * 4). Fires only for `outputProcessors`-mounted instances and runs AFTER
     * `processLLMResponse` on streamed steps (V4 pin), so in the common case
     * the reservation is already settled and the inflight pop comes back
     * empty — that is the at-most-one-commit guard, not an error (silent
     * no-op; TP-31). It settles for real only when an open reservation
     * reaches this hook unsettled (e.g. a dual-mounted instance whose
     * response-hook settlement did not run); an output-mounted-ONLY instance
     * never reserves, so its backstop pop always no-ops.
     */
    processOutputStep(args: ProcessOutputStepArgs): Promise<MessageList>;
    /**
     * FAILURE COMMIT — V7 SECONDARY signal (pin header): non-retryable API
     * rejections surfaced through the installed `processAPIError` hook
     * (design §6.1 row 5). The FIFO pop dedups against the response hook's
     * error-chunk settlement. Never requests a retry and never throws past
     * the commit swallow: the ORIGINAL provider error must propagate to the
     * consumer (design §7 commit rows).
     */
    processAPIError(args: ProcessAPIErrorArgs): Promise<undefined>;
    /**
     * No-op in v1 (design §11.3 LOCKED): the reserve already brackets the
     * step at `processInputStep`. Kept as the pinned fallback reserve point
     * if a model path ever skips `processInputStep` (V1 register note); any
     * reserve logic here is drift.
     */
    processLLMRequest(_args: ProcessLLMRequestArgs): undefined;
    /** §6.4 LOCKED default claim projection (D04/D06 parity). */
    private projectClaim;
    private buildUnit;
    /**
     * Recover the §6.5 inflight key at a commit hook (V4 pin header) and pop
     * the oldest open entry for it. Key sources, in order: the state-stashed
     * per-step runId, then the consumer's runIdProvider. No key / no entry →
     * undefined (caller decides warn vs silent backstop no-op).
     */
    private popInflight;
    /**
     * Emit the settlement `commitEstimated` for a popped reservation —
     * tuple-matched to the reserve (HARDEN_D05_WI): same identity tuple, same
     * unit, same pricing freeze (`opts.pricing` stash; empty tuple when the
     * option is absent — design §6.7 amendment #3).
     *
     *   - SUCCESS + usage: actuals on the wire fields; estimate = token sum
     *     (shipped-D04-handler wire shape, HARDEN_D05_WI — the ledger rejects
     *     `estimated_amount_atomic = 0` bookings).
     *   - SUCCESS without usage (§6.6 LOCKED fallback): estimate =
     *     reserve-time `projectedAmountAtomic`; actuals OMITTED — the audit
     *     chain records that no provider actuals were observed.
     *   - FAILURE: estimate = reserve-time projection (usage is absent on the
     *     error path — same §6.6 rule), `actualErrorMessage` threaded.
     *
     * Commit RPC errors are swallowed at error level (§7.4 LOCKED asymmetry;
     * sidecar TTL sweep + audit chain settle the reservation). KNOWN drift,
     * absorbed: the sidecar may reject the outcome COMPANION event with
     * "missing estimated_amount_atomic" — the booking still lands; the
     * warn-not-throw path covers it (do not chase).
     */
    private settleCommit;
}

declare const VERSION: "0.1.1";

export { type ClaimEstimator, type ClaimEstimatorInput, SpendGuardProcessor, type SpendGuardProcessorOptions, VERSION };
