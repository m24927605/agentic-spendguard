// The five-event SpendGuard AG-UI vocabulary — design.md §5.2, LOCKED.
//
// These five strings are the public vocabulary. Renames, additions, or
// removals after the spec merge require a design.md re-spec
// (review-standards §2 P0). No sixth name exists anywhere.
export const SPENDGUARD_AG_UI_EVENT_NAMES = {
  budgetSnapshot: "spendguard.budget.snapshot",
  reservationCreated: "spendguard.reservation.created",
  reservationCommitted: "spendguard.reservation.committed",
  reservationReleased: "spendguard.reservation.released",
  decisionDenied: "spendguard.decision.denied",
} as const;

export type SpendGuardAgUiEventName =
  (typeof SPENDGUARD_AG_UI_EVENT_NAMES)[keyof typeof SPENDGUARD_AG_UI_EVENT_NAMES];
