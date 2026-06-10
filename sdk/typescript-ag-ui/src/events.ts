// Envelope + builder-input types — design.md §8.1, LOCKED (verbatim).
//
// The envelope is OUR type, structurally matching the AG-UI CUSTOM event
// shape (design.md §10.1): no @ag-ui/core import appears anywhere in src/.
//
// [VERIFY-AT-IMPL resolved 2026-06-10 — design.md §5.1 envelope timestamp]
// Confirmed against the pinned @ag-ui/core@0.0.56 (and the oldest published
// 0.0.27): `BaseEventSchema` / `CustomEventSchema` carry an OPTIONAL field
// named exactly `timestamp` typed `z.ZodOptional<z.ZodNumber>`. Epoch-ms
// semantics confirmed by AG-UI's first-party docs middleware example
// (docs/sdk/js/client/middleware.mdx of ag-ui-protocol/ag-ui stamps the
// current epoch-millisecond clock value into the `timestamp` field) and by
// @ag-ui/proto's int64 wire encoding. The §5.1
// envelope key `timestamp` therefore stands as designed — no design.md
// revision needed.
import type { SpendGuardAgUiEventName } from "./names.js";

// ── Envelope ────────────────────────────────────────────────────────────
export interface SpendGuardAgUiEvent {
  readonly type: "CUSTOM";
  readonly name: SpendGuardAgUiEventName;
  readonly value: Readonly<Record<string, unknown>>;
  readonly timestamp?: number; // integer epoch ms; present iff caller supplied
}

export interface BuildContext {
  /** AG-UI envelope timestamp (integer epoch milliseconds). Builders never
   *  read clocks: omitted from the event when not provided. */
  timestampMs?: number;
}

// ── Builder inputs (camelCase per TS house style; builders map to the
//    snake_case payload keys locked in design.md §5) ─────────────────────
export interface BudgetSnapshotInput {
  budgetId: string;
  windowInstanceId: string;
  unit: string;
  unitId?: string;
  remainingAtomic: string;
  reservedAtomic: string;
  spentAtomic: string;
  asOf: string; // RFC 3339
}

export interface ReservationCreatedInput {
  decisionId: string;
  reservationId: string;
  budgetId: string;
  windowInstanceId: string;
  unit: string;
  unitId?: string;
  amountAtomicReserved: string;
  decision: "ALLOW" | "ALLOW_WITH_CAPS";
  ttlExpiresAt: string; // RFC 3339
  reasonCodes?: readonly string[];
  matchedRuleIds?: readonly string[];
  runId?: string;
  llmCallId?: string;
  eventTime: string; // RFC 3339
}

export interface ReservationCommittedInput {
  decisionId: string;
  reservationId: string;
  budgetId: string;
  windowInstanceId: string;
  unit: string;
  unitId?: string;
  amountAtomicEstimated: string;
  amountAtomicObserved?: string; // reserved — future observed commit lane
  outcome: "SUCCESS" | "PROVIDER_ERROR" | "CLIENT_TIMEOUT" | "RUN_ABORTED";
  runId?: string;
  llmCallId?: string;
  eventTime: string; // RFC 3339
}

export interface ReservationReleasedInput {
  reservationId: string;
  decisionId?: string;
  reasonCodes: readonly string[]; // ≥ 1 entry
  ledgerTransactionId?: string;
  runId?: string;
  llmCallId?: string;
  eventTime: string; // RFC 3339
}

export interface DecisionDeniedInput {
  decisionId: string;
  deniedKind: "DENY" | "STOP" | "STOP_RUN_PROJECTION" | "SKIP" | "APPROVAL_REQUIRED";
  reasonCodes: readonly string[]; // ≥ 1 entry; APPROVAL_REQUIRED ⇒ must include "approval_required"
  matchedRuleIds?: readonly string[];
  budgetId?: string;
  windowInstanceId?: string;
  unit?: string;
  unitId?: string;
  runId?: string;
  llmCallId?: string;
  eventTime: string; // RFC 3339
}
