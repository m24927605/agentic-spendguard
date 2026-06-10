// src/index.ts — public barrel of @spendguard/mastra. Named exports only.
//
// COV_D38_01 placeholder: a strict SUBSET of the design.md §5 LOCKED barrel
// (VERSION + the three error re-exports). `SpendGuardProcessor` and the
// options types land in COV_D38_02, completing the §5 verbatim shape. This
// barrel never contains anything NOT in §5.

export { DecisionDenied, SidecarUnavailable, SpendGuardError } from "./errors.js";
export { VERSION } from "./version.js";
