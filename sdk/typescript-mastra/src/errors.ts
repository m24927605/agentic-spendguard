// Adapter error surface — re-exports the substrate's typed errors so
// consumers can pattern-match on them without taking a second
// `@spendguard/sdk` import (implementation.md §3.6, copied verbatim).
//
// Exactly the D06 three-class anti-list. Other substrate errors
// (`DecisionStopped`, `ApprovalRequired`, `HandshakeError`, …) remain
// importable from `@spendguard/sdk` directly — note `DecisionStopped` /
// `ApprovalRequired` are subclasses of `DecisionDenied`, so
// `instanceof DecisionDenied` catches all denial flavours.
//
// Direct re-export: class identity is preserved, so `instanceof` works
// across the adapter ↔ substrate boundary.
export { DecisionDenied, SidecarUnavailable, SpendGuardError } from "@spendguard/sdk";
