import { Processor, ProcessInputStepArgs, ProcessLLMRequestArgs } from '@mastra/core/processors';
import { BudgetClaim, SpendGuardClient } from '@spendguard/sdk';
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
     * No-op in v1 (design §11.3 LOCKED): the reserve already brackets the
     * step at `processInputStep`. Kept as the pinned fallback reserve point
     * if a model path ever skips `processInputStep` (V1 register note); any
     * reserve logic here is drift.
     */
    processLLMRequest(_args: ProcessLLMRequestArgs): undefined;
    /** §6.4 LOCKED default claim projection (D04/D06 parity). */
    private projectClaim;
    private buildUnit;
}

declare const VERSION: "0.1.0";

export { type ClaimEstimator, type ClaimEstimatorInput, SpendGuardProcessor, type SpendGuardProcessorOptions, VERSION };
