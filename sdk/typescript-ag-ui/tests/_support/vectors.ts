// Shared deterministic input vectors for the @spendguard/ag-ui test suite.
//
// Every value is a fixed literal — no clocks, no RNG (design.md §11.3).
// IDs are substrate-shaped inputs (UUIDv7-style), never minted here
// (design.md §11.6).
import type {
  BudgetSnapshotInput,
  DecisionDeniedInput,
  ReservationCommittedInput,
  ReservationCreatedInput,
  ReservationReleasedInput,
} from "../../src/events.js";

export const TS_MS = 1765843200000;

export const SNAPSHOT_MIN: BudgetSnapshotInput = {
  budgetId: "budget-dev-monthly",
  windowInstanceId: "0197a001-0000-7000-8000-00000000win1",
  unit: "usd_micros",
  remainingAtomic: "25000000",
  reservedAtomic: "0",
  spentAtomic: "0",
  asOf: "2026-06-10T07:59:00Z",
};

export const SNAPSHOT_MAX: BudgetSnapshotInput = {
  ...SNAPSHOT_MIN,
  unitId: "0197a001-2222-7000-8000-0000000unit1",
};

export const CREATED_MIN: ReservationCreatedInput = {
  decisionId: "0197a001-aaaa-7000-8000-000000000d01",
  reservationId: "0197a001-bbbb-7000-8000-000000000r01",
  budgetId: "budget-dev-monthly",
  windowInstanceId: "0197a001-0000-7000-8000-00000000win1",
  unit: "usd_micros",
  amountAtomicReserved: "1000000",
  decision: "ALLOW",
  ttlExpiresAt: "2026-06-10T08:00:00Z",
  eventTime: "2026-06-10T07:59:58Z",
};

export const CREATED_MAX: ReservationCreatedInput = {
  ...CREATED_MIN,
  unitId: "0197a001-2222-7000-8000-0000000unit1",
  decision: "ALLOW_WITH_CAPS",
  reasonCodes: ["degrade_applied", "model_capped"],
  matchedRuleIds: ["rule-cap-model", "rule-budget-soft"],
  runId: "0197a001-cccc-7000-8000-00000000run1",
  llmCallId: "0197a001-dddd-7000-8000-0000000call1",
};

export const COMMITTED_MIN: ReservationCommittedInput = {
  decisionId: "0197a001-aaaa-7000-8000-000000000d01",
  reservationId: "0197a001-bbbb-7000-8000-000000000r01",
  budgetId: "budget-dev-monthly",
  windowInstanceId: "0197a001-0000-7000-8000-00000000win1",
  unit: "usd_micros",
  amountAtomicEstimated: "950000",
  outcome: "SUCCESS",
  eventTime: "2026-06-10T08:00:02Z",
};

export const COMMITTED_MAX: ReservationCommittedInput = {
  ...COMMITTED_MIN,
  unitId: "0197a001-2222-7000-8000-0000000unit1",
  amountAtomicObserved: "940123",
  runId: "0197a001-cccc-7000-8000-00000000run1",
  llmCallId: "0197a001-dddd-7000-8000-0000000call1",
};

export const RELEASED_MIN: ReservationReleasedInput = {
  reservationId: "0197a001-bbbb-7000-8000-000000000r01",
  reasonCodes: ["client_timeout"],
  eventTime: "2026-06-10T08:00:30Z",
};

export const RELEASED_MAX: ReservationReleasedInput = {
  ...RELEASED_MIN,
  decisionId: "0197a001-aaaa-7000-8000-000000000d01",
  reasonCodes: ["provider_error", "run_cancelled"],
  ledgerTransactionId: "0197a001-eeee-7000-8000-000000000tx1",
  runId: "0197a001-cccc-7000-8000-00000000run1",
  llmCallId: "0197a001-dddd-7000-8000-0000000call1",
};

export const DENIED_MIN: DecisionDeniedInput = {
  decisionId: "0197a001-ffff-7000-8000-000000000d02",
  deniedKind: "DENY",
  reasonCodes: ["BUDGET_EXHAUSTED"],
  eventTime: "2026-06-10T08:01:00Z",
};

export const DENIED_MAX: DecisionDeniedInput = {
  ...DENIED_MIN,
  deniedKind: "STOP_RUN_PROJECTION",
  reasonCodes: ["RUN_BUDGET_PROJECTION_EXCEEDED", "RUN_CEILING"],
  matchedRuleIds: ["rule-run-projection"],
  budgetId: "budget-dev-monthly",
  windowInstanceId: "0197a001-0000-7000-8000-00000000win1",
  unit: "usd_micros",
  unitId: "0197a001-2222-7000-8000-0000000unit1",
  runId: "0197a001-cccc-7000-8000-00000000run1",
  llmCallId: "0197a001-dddd-7000-8000-0000000call1",
};

/** Recursively freeze an object graph (TP-12 input-mutation guard). */
export function deepFreeze<T>(obj: T): T {
  if (obj !== null && typeof obj === "object") {
    for (const v of Object.values(obj)) {
      deepFreeze(v);
    }
    Object.freeze(obj);
  }
  return obj;
}
