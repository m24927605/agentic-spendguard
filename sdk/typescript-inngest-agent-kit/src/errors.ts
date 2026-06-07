// Adapter error surface — re-exports the substrate's typed errors so consumers
// can pattern-match on them without taking a second `@spendguard/sdk` import.
//
// D29 re-exports a wider error set than D04 because the retry-dedup contract
// (review-standards §4) plus the ApprovalRequired resume path (review-standards
// §5.4) both surface specific typed errors callers must distinguish in the
// step body. The class identity is preserved via direct re-export so
// `err instanceof DecisionDenied` keeps working across the adapter ↔ substrate
// boundary.

export {
  ApprovalRequired,
  DecisionDenied,
  DecisionSkipped,
  DecisionStopped,
  SidecarUnavailable,
  SpendGuardError,
} from "@spendguard/sdk";
