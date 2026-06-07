// Adapter error surface — re-exports the substrate's typed errors so consumers
// can pattern-match on them without taking a second `@spendguard/sdk` import.
//
// SLICE 2 deliberately re-exports only the three classes the locked surface
// names (design.md §4 + review-standards.md §1.8 anti-list). Other substrate
// errors (`DecisionStopped`, `ApprovalRequired`, `MutationApplyFailed`, …)
// remain importable from `@spendguard/sdk` directly — consumers reach for them
// in `try/catch` blocks, not through the adapter barrel.
//
// SLICE 3 will wire `reserve` / `commitEstimated`; these errors then start
// flowing out of `handleChatModelStart` / `handleLLMEnd`. The class identity is
// preserved via direct re-export so `err instanceof DecisionDenied` keeps
// working across the adapter ↔ substrate boundary.

export { DecisionDenied, SidecarUnavailable, SpendGuardError } from "@spendguard/sdk";
