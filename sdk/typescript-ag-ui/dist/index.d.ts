declare const SPENDGUARD_AG_UI_EVENT_NAMES: {
    readonly budgetSnapshot: "spendguard.budget.snapshot";
    readonly reservationCreated: "spendguard.reservation.created";
    readonly reservationCommitted: "spendguard.reservation.committed";
    readonly reservationReleased: "spendguard.reservation.released";
    readonly decisionDenied: "spendguard.decision.denied";
};
type SpendGuardAgUiEventName = (typeof SPENDGUARD_AG_UI_EVENT_NAMES)[keyof typeof SPENDGUARD_AG_UI_EVENT_NAMES];

interface SpendGuardAgUiEvent {
    readonly type: "CUSTOM";
    readonly name: SpendGuardAgUiEventName;
    readonly value: Readonly<Record<string, unknown>>;
    readonly timestamp?: number;
}
interface BuildContext {
    /** AG-UI envelope timestamp (integer epoch milliseconds). Builders never
     *  read clocks: omitted from the event when not provided. */
    timestampMs?: number;
}
interface BudgetSnapshotInput {
    budgetId: string;
    windowInstanceId: string;
    unit: string;
    unitId?: string;
    remainingAtomic: string;
    reservedAtomic: string;
    spentAtomic: string;
    asOf: string;
}
interface ReservationCreatedInput {
    decisionId: string;
    reservationId: string;
    budgetId: string;
    windowInstanceId: string;
    unit: string;
    unitId?: string;
    amountAtomicReserved: string;
    decision: "ALLOW" | "ALLOW_WITH_CAPS";
    ttlExpiresAt: string;
    reasonCodes?: readonly string[];
    matchedRuleIds?: readonly string[];
    runId?: string;
    llmCallId?: string;
    eventTime: string;
}
interface ReservationCommittedInput {
    decisionId: string;
    reservationId: string;
    budgetId: string;
    windowInstanceId: string;
    unit: string;
    unitId?: string;
    amountAtomicEstimated: string;
    amountAtomicObserved?: string;
    outcome: "SUCCESS" | "PROVIDER_ERROR" | "CLIENT_TIMEOUT" | "RUN_ABORTED";
    runId?: string;
    llmCallId?: string;
    eventTime: string;
}
interface ReservationReleasedInput {
    reservationId: string;
    decisionId?: string;
    reasonCodes: readonly string[];
    ledgerTransactionId?: string;
    runId?: string;
    llmCallId?: string;
    eventTime: string;
}
interface DecisionDeniedInput {
    decisionId: string;
    deniedKind: "DENY" | "STOP" | "STOP_RUN_PROJECTION" | "SKIP" | "APPROVAL_REQUIRED";
    reasonCodes: readonly string[];
    matchedRuleIds?: readonly string[];
    budgetId?: string;
    windowInstanceId?: string;
    unit?: string;
    unitId?: string;
    runId?: string;
    llmCallId?: string;
    eventTime: string;
}

declare function buildBudgetSnapshot(input: BudgetSnapshotInput, ctx?: BuildContext): SpendGuardAgUiEvent;
declare function buildReservationCreated(input: ReservationCreatedInput, ctx?: BuildContext): SpendGuardAgUiEvent;
declare function buildReservationCommitted(input: ReservationCommittedInput, ctx?: BuildContext): SpendGuardAgUiEvent;
declare function buildReservationReleased(input: ReservationReleasedInput, ctx?: BuildContext): SpendGuardAgUiEvent;
declare function buildDecisionDenied(input: DecisionDeniedInput, ctx?: BuildContext): SpendGuardAgUiEvent;

declare function canonicalEventJson(event: SpendGuardAgUiEvent): string;

declare function encodeSse(event: SpendGuardAgUiEvent): string;
type AgUiEmit = (event: SpendGuardAgUiEvent) => void | Promise<void>;

declare class AgUiEventValidationError extends Error {
    readonly field: string;
    constructor(field: string, message?: string);
}

declare const VERSION: "0.1.0";

export { type AgUiEmit, AgUiEventValidationError, type BudgetSnapshotInput, type BuildContext, type DecisionDeniedInput, type ReservationCommittedInput, type ReservationCreatedInput, type ReservationReleasedInput, SPENDGUARD_AG_UI_EVENT_NAMES, type SpendGuardAgUiEvent, type SpendGuardAgUiEventName, VERSION, buildBudgetSnapshot, buildDecisionDenied, buildReservationCommitted, buildReservationCreated, buildReservationReleased, canonicalEventJson, encodeSse };
