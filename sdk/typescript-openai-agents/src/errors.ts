// Adapter error surface — re-exports the substrate's typed errors so consumers
// can pattern-match on them without taking a second `@spendguard/sdk` import.
//
// SLICE 2 ships only the four classes the locked surface names (design.md §4
// + review-standards.md §10 / §13 anti-list). Other substrate errors
// (`DecisionSkipped`, `MutationApplyFailed`, …) remain importable from
// `@spendguard/sdk` directly — consumers reach for them in `try/catch` blocks,
// not through the adapter barrel.
//
// `withSpendGuard` / `SpendGuardAgentsModel` (slice 2) calls `client.reserve()`
// inside `bracketedGetResponse`. The substrate-typed errors propagate UNCHANGED
// — `DecisionDenied` / `DecisionStopped` / `ApprovalRequired` are caught by
// the OpenAI Agents `Runner.run` consumer downstream. `SidecarUnavailable` is
// the operational-degradation signal documented in `review-standards.md`
// §10.1; the adapter does NOT catch it — Runner caller decides if a sidecar
// outage halts the run (parity with Python wrapper). The future
// `degrade=auto` mode (LOCKED OUT of v0.1.x per design §3) would catch it
// here and pass through; that would be a v0.2 minor.
//
// Class identity is preserved via DIRECT RE-EXPORT (no wrapping) so
// `err instanceof DecisionDenied` keeps working across the adapter ↔
// substrate boundary.

export {
  DecisionDenied,
  DecisionStopped,
  ApprovalRequired,
  SidecarUnavailable,
  SpendGuardError,
} from "@spendguard/sdk";
