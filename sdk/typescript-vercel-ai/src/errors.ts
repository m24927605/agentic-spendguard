// Adapter error surface — re-exports the substrate's typed errors so consumers
// can pattern-match on them without taking a second `@spendguard/sdk` import.
//
// SLICE 2 deliberately re-exports only the three classes the locked surface
// names (design.md §4 + review-standards.md §1.8 anti-list). Other substrate
// errors (`DecisionStopped`, `ApprovalRequired`, `MutationApplyFailed`, …)
// remain importable from `@spendguard/sdk` directly — consumers reach for them
// in `try/catch` blocks, not through the adapter barrel.
//
// SLICE 3 wires `client.reserve()` inside `transformParams`. `DecisionDenied`
// (and its `DecisionStopped` / `ApprovalRequired` subclasses) propagate
// out of `transformParams` so the Vercel AI SDK caller sees the typed error.
// `SidecarUnavailable` is swallowed inside `transformParams` so a sidecar
// outage does NOT block the LLM call — see middleware.ts for the
// "operational degradation, not enforcement" policy.
//
// SLICE 4 (`wrapGenerate`) + SLICE 5 (`wrapStream`) will start emitting
// commit / release calls; the same error-surface re-export keeps holding.
// The class identity is preserved via direct re-export so
// `err instanceof DecisionDenied` keeps working across the adapter ↔
// substrate boundary.

export { DecisionDenied, SidecarUnavailable, SpendGuardError } from "@spendguard/sdk";
