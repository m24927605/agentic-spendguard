// @spendguard/ag-ui — SpendGuard spend-event family for AG-UI.
//
// **Display-only.** AG-UI events are a presentation surface. SpendGuard
// enforcement happens in the SpendGuard adapters and sidecar before the
// provider call; these events report decisions already made and can neither
// grant nor deny spend.
//
// Public barrel — design.md §8.1, nothing else. No other exports. No
// default export. NO re-export of @spendguard/sdk or @ag-ui/core symbols
// (the package depends on neither at runtime).
export { SPENDGUARD_AG_UI_EVENT_NAMES } from "./names.js";
export type { SpendGuardAgUiEventName } from "./names.js";
export type {
  BudgetSnapshotInput,
  BuildContext,
  DecisionDeniedInput,
  ReservationCommittedInput,
  ReservationCreatedInput,
  ReservationReleasedInput,
  SpendGuardAgUiEvent,
} from "./events.js";
export {
  buildBudgetSnapshot,
  buildDecisionDenied,
  buildReservationCommitted,
  buildReservationCreated,
  buildReservationReleased,
} from "./builders.js";
export { canonicalEventJson } from "./canonical.js";
export type { AgUiEmit } from "./sse.js";
export { encodeSse } from "./sse.js";
export { AgUiEventValidationError } from "./errors.js";
export { VERSION } from "./version.js";
