// @spendguard/langchain — LangChain TS adapter for SpendGuard budget guardrails.
//
// SLICE 1 shipped the package skeleton. SLICE 2 adds the
// `SpendGuardCallbackHandler` class shape + LOCKED options surface; PRE/POST
// hooks throw `"SLICE 3 not implemented"` until SLICE 3 wires reserve/commit.
// Full docs page lands in SLICE 6.
//
// Public surface (LOCKED per design.md §4 / review-standards.md §1):
//   - `SpendGuardCallbackHandler`           — the BaseCallbackHandler subclass.
//   - `SpendGuardCallbackHandlerOptions`    — the constructor option shape.
//   - `SpendGuardError` / `DecisionDenied` / `SidecarUnavailable`
//                                            — re-exports from @spendguard/sdk
//                                              so `err instanceof DecisionDenied`
//                                              works without the consumer
//                                              taking a second import.
//   - `VERSION`                             — package version constant.
//
// No `default` export — review-standards.md §1.7.

export { SpendGuardCallbackHandler } from "./handler.js";
export type { SpendGuardCallbackHandlerOptions } from "./options.js";
export { DecisionDenied, SidecarUnavailable, SpendGuardError } from "./errors.js";
export { VERSION } from "./version.js";
