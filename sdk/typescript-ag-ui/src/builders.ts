import { AgUiEventValidationError } from "./errors.js";
// The five pure builders — design.md §5.3-§5.7 payload schemas (LOCKED) +
// §8.1 signatures (LOCKED).
//
// Purity contract (design.md §4, §11.3): no clock reads, no RNG, no env, no
// I/O, no global state. `event_time` / `as_of` / `timestampMs` are inputs.
// Every ID is an input — builders never mint or hash (design.md §11.6).
import type {
  BudgetSnapshotInput,
  BuildContext,
  DecisionDeniedInput,
  ReservationCommittedInput,
  ReservationCreatedInput,
  ReservationReleasedInput,
  SpendGuardAgUiEvent,
} from "./events.js";
import { SPENDGUARD_AG_UI_EVENT_NAMES } from "./names.js";
import {
  optionalEntry,
  requireAtomic,
  requireNonEmpty,
  requireRfc3339,
  requireSafeInteger,
  requireStringArray,
} from "./validate.js";

// SpendGuard wire mapping (design.md §5.4): CONTINUE → ALLOW;
// DEGRADE → ALLOW_WITH_CAPS (ASP Draft-01 §2 pattern). The mapping is the
// CALLER's: builders accept only the ASP enum verbatim.
const CREATED_DECISIONS: readonly string[] = ["ALLOW", "ALLOW_WITH_CAPS"];

// SpendGuard `CommitEstimatedRequest.outcome` enum, verbatim, all four
// values (design.md §5.5).
const COMMITTED_OUTCOMES: readonly string[] = [
  "SUCCESS",
  "PROVIDER_ERROR",
  "CLIENT_TIMEOUT",
  "RUN_ABORTED",
];

// SpendGuard sidecar decision-outcome taxonomy (design.md §5.7).
const DENIED_KINDS: readonly string[] = [
  "DENY",
  "STOP",
  "STOP_RUN_PROJECTION",
  "SKIP",
  "APPROVAL_REQUIRED",
];

/** Assemble the envelope. Purity contract: no clock reads, no randomness.
 *  `timestamp` is present iff the caller supplied `ctx.timestampMs` —
 *  `0` is a valid epoch ms and IS emitted when explicitly provided. */
function envelope(
  name: SpendGuardAgUiEvent["name"],
  value: Record<string, unknown>,
  ctx?: BuildContext,
): SpendGuardAgUiEvent {
  if (ctx?.timestampMs !== undefined) {
    requireSafeInteger("timestamp", ctx.timestampMs);
    return Object.freeze({
      type: "CUSTOM" as const,
      name,
      value: Object.freeze(value),
      timestamp: ctx.timestampMs,
    });
  }
  return Object.freeze({ type: "CUSTOM" as const, name, value: Object.freeze(value) });
}

/** Required string[] payload entry: validated (>= 1 entry of non-empty
 *  strings), copied so later caller mutation cannot reach the event, and
 *  frozen. Array order is preserved as given (design.md §7.6). */
function requiredArrayEntry(field: string, a: readonly string[]): readonly string[] {
  return Object.freeze([...requireStringArray(field, a, { minLen: 1 })]);
}

/** Optional string[] payload entry: emitted only when provided AND
 *  non-empty (design.md §5.4/§5.7 omit-if-absent/empty); entries must be
 *  non-empty strings when emitted. */
function optionalArrayEntry(
  field: string,
  a: readonly string[] | undefined,
): Record<string, readonly string[]> {
  if (a === undefined || (Array.isArray(a) && a.length === 0)) {
    return {};
  }
  return { [field]: Object.freeze([...requireStringArray(field, a, { minLen: 1 })]) };
}

// ── spendguard.budget.snapshot — design.md §5.3 ─────────────────────────
export function buildBudgetSnapshot(
  input: BudgetSnapshotInput,
  ctx?: BuildContext,
): SpendGuardAgUiEvent {
  const value: Record<string, unknown> = {
    schema_version: "1",
    budget_id: requireNonEmpty("budget_id", input.budgetId),
    window_instance_id: requireNonEmpty("window_instance_id", input.windowInstanceId),
    unit: requireNonEmpty("unit", input.unit),
    remaining_atomic: requireAtomic("remaining_atomic", input.remainingAtomic),
    reserved_atomic: requireAtomic("reserved_atomic", input.reservedAtomic),
    spent_atomic: requireAtomic("spent_atomic", input.spentAtomic),
    as_of: requireRfc3339("as_of", input.asOf),
    ...optionalEntry("unit_id", input.unitId), // omit-if-empty (design §6)
  };
  return envelope(SPENDGUARD_AG_UI_EVENT_NAMES.budgetSnapshot, value, ctx);
}

// ── spendguard.reservation.created — design.md §5.4 ─────────────────────
export function buildReservationCreated(
  input: ReservationCreatedInput,
  ctx?: BuildContext,
): SpendGuardAgUiEvent {
  if (!CREATED_DECISIONS.includes(input.decision)) {
    throw new AgUiEventValidationError(
      "decision",
      'field "decision" must be "ALLOW" or "ALLOW_WITH_CAPS" (ASP decision enum, design.md §5.4)',
    );
  }
  const value: Record<string, unknown> = {
    schema_version: "1",
    decision_id: requireNonEmpty("decision_id", input.decisionId),
    reservation_id: requireNonEmpty("reservation_id", input.reservationId),
    budget_id: requireNonEmpty("budget_id", input.budgetId),
    window_instance_id: requireNonEmpty("window_instance_id", input.windowInstanceId),
    unit: requireNonEmpty("unit", input.unit),
    amount_atomic_reserved: requireAtomic("amount_atomic_reserved", input.amountAtomicReserved),
    decision: input.decision,
    ttl_expires_at: requireRfc3339("ttl_expires_at", input.ttlExpiresAt),
    event_time: requireRfc3339("event_time", input.eventTime),
    ...optionalEntry("unit_id", input.unitId), // §6 unitId invariant
    ...optionalArrayEntry("reason_codes", input.reasonCodes),
    ...optionalArrayEntry("matched_rule_ids", input.matchedRuleIds),
    ...optionalEntry("run_id", input.runId),
    ...optionalEntry("llm_call_id", input.llmCallId),
  };
  return envelope(SPENDGUARD_AG_UI_EVENT_NAMES.reservationCreated, value, ctx);
}

// ── spendguard.reservation.committed — design.md §5.5 ───────────────────
export function buildReservationCommitted(
  input: ReservationCommittedInput,
  ctx?: BuildContext,
): SpendGuardAgUiEvent {
  if (!COMMITTED_OUTCOMES.includes(input.outcome)) {
    throw new AgUiEventValidationError(
      "outcome",
      'field "outcome" must be one of "SUCCESS" | "PROVIDER_ERROR" | "CLIENT_TIMEOUT" | "RUN_ABORTED" (design.md §5.5)',
    );
  }
  const value: Record<string, unknown> = {
    schema_version: "1",
    decision_id: requireNonEmpty("decision_id", input.decisionId),
    reservation_id: requireNonEmpty("reservation_id", input.reservationId),
    budget_id: requireNonEmpty("budget_id", input.budgetId),
    window_instance_id: requireNonEmpty("window_instance_id", input.windowInstanceId),
    unit: requireNonEmpty("unit", input.unit),
    // SpendGuard extension, documented delta (design.md §5.5): the only
    // commit lane today is CommitEstimated — named distinctly from ASP's
    // amount_atomic_observed so no consumer mistakes it for provider-
    // reported usage.
    amount_atomic_estimated: requireAtomic("amount_atomic_estimated", input.amountAtomicEstimated),
    outcome: input.outcome,
    event_time: requireRfc3339("event_time", input.eventTime),
    ...optionalEntry("unit_id", input.unitId), // §6 unitId invariant
    // Reserved ASP field (design.md §5.5): omit-if-ABSENT — when supplied it
    // must be a valid atomic decimal string (the §6 empty≡absent collapse is
    // scoped to optional ID-style string fields; an amount is validated).
    ...(input.amountAtomicObserved !== undefined
      ? {
          amount_atomic_observed: requireAtomic(
            "amount_atomic_observed",
            input.amountAtomicObserved,
          ),
        }
      : {}),
    ...optionalEntry("run_id", input.runId),
    ...optionalEntry("llm_call_id", input.llmCallId),
  };
  return envelope(SPENDGUARD_AG_UI_EVENT_NAMES.reservationCommitted, value, ctx);
}

// ── spendguard.reservation.released — design.md §5.6 ────────────────────
export function buildReservationReleased(
  input: ReservationReleasedInput,
  ctx?: BuildContext,
): SpendGuardAgUiEvent {
  const value: Record<string, unknown> = {
    schema_version: "1",
    reservation_id: requireNonEmpty("reservation_id", input.reservationId),
    // REQUIRED with >= 1 entry — "Reason for release goes in reason_codes"
    // (ASP audit.release, design.md §5.6).
    reason_codes: requiredArrayEntry("reason_codes", input.reasonCodes),
    event_time: requireRfc3339("event_time", input.eventTime),
    // Optional here because the adapter-wire ReleaseRequest does not carry
    // it (design.md §5.6).
    ...optionalEntry("decision_id", input.decisionId),
    ...optionalEntry("ledger_transaction_id", input.ledgerTransactionId),
    ...optionalEntry("run_id", input.runId),
    ...optionalEntry("llm_call_id", input.llmCallId),
  };
  return envelope(SPENDGUARD_AG_UI_EVENT_NAMES.reservationReleased, value, ctx);
}

// ── spendguard.decision.denied — design.md §5.7 ─────────────────────────
export function buildDecisionDenied(
  input: DecisionDeniedInput,
  ctx?: BuildContext,
): SpendGuardAgUiEvent {
  if (!DENIED_KINDS.includes(input.deniedKind)) {
    throw new AgUiEventValidationError(
      "denied_kind",
      'field "denied_kind" must be one of "DENY" | "STOP" | "STOP_RUN_PROJECTION" | "SKIP" | "APPROVAL_REQUIRED" (design.md §5.7)',
    );
  }
  const reasonCodes = requireStringArray("reason_codes", input.reasonCodes, { minLen: 1 });
  // ASP Draft-01 §2: approval-required is DENY + the "approval_required"
  // reason code. The builder validates and throws — it does NOT silently
  // append (design.md §5.7).
  if (input.deniedKind === "APPROVAL_REQUIRED" && !reasonCodes.includes("approval_required")) {
    throw new AgUiEventValidationError(
      "reason_codes",
      'denied_kind APPROVAL_REQUIRED requires reason_codes to include "approval_required" (ASP Draft-01 §2)',
    );
  }
  const value: Record<string, unknown> = {
    schema_version: "1",
    decision_id: requireNonEmpty("decision_id", input.decisionId),
    // Injected literal — every deny-class SpendGuard outcome is ASP DENY
    // (design.md §5.7).
    decision: "DENY",
    denied_kind: input.deniedKind,
    reason_codes: Object.freeze([...reasonCodes]),
    event_time: requireRfc3339("event_time", input.eventTime),
    ...optionalArrayEntry("matched_rule_ids", input.matchedRuleIds),
    ...optionalEntry("budget_id", input.budgetId), // a deny can fire before budget binding
    ...optionalEntry("window_instance_id", input.windowInstanceId),
    ...optionalEntry("unit", input.unit),
    ...optionalEntry("unit_id", input.unitId), // §6 unitId invariant
    ...optionalEntry("run_id", input.runId),
    ...optionalEntry("llm_call_id", input.llmCallId),
  };
  return envelope(SPENDGUARD_AG_UI_EVENT_NAMES.decisionDenied, value, ctx);
}
