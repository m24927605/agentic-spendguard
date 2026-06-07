// @spendguard/openai-agents — OpenAI Agents SDK (TypeScript) adapter for
// SpendGuard budget guardrails.
//
// SLICE 1 + 2 ships the package skeleton + `withSpendGuard` factory +
// `SpendGuardAgentsModel` subclass + `runContext()` AsyncLocalStorage
// shim + cross-language signature helper. SLICE 3 lands the cross-language
// fixture extension (`sdk/fixtures/cross-language/v1.json#openai_agents`)
// + default `claimEstimator` derived from `inner.model`. SLICE 4-5 wire
// the `examples/openai-agents-ts-composite/` demo. SLICE 6 ships the
// publish workflow.
//
// Public surface (LOCKED per design.md §4 + review-standards.md §3):
//   - `withSpendGuard(inner, opts): Model`                         — SLICE 2
//   - `SpendGuardAgentsModel`                                      — SLICE 2
//   - `runContext` / `currentRunContext` / `RunContext`            — SLICE 2
//   - `SpendGuardAgentsOptions`                                    — SLICE 2
//   - `deriveAgentSignature`                                       — SLICE 2
//   - `extractUsage` / `ExtractedUsage`                            — SLICE 2
//   - `DecisionDenied` / `DecisionStopped` / `ApprovalRequired` /
//     `SidecarUnavailable` / `SpendGuardError`                     — SLICE 2
//   - `VERSION`                                                    — SLICE 1
//
// No `default` export — review-standards.md §3.5.
//
// Sibling subpath entry `./run-context` re-exports the AsyncLocalStorage
// API so a sibling package (D04 / D06 / D29) can read the SAME storage
// without taking the main barrel as a dep. tsup `splitting: true` shares
// the underlying chunk so the `Symbol.for(...)` key still resolves to the
// one and only storage at runtime (design.md §7 decision #4).

export { withSpendGuard } from "./withSpendGuard.js";
export { SpendGuardAgentsModel } from "./model.js";
export { runContext, currentRunContext } from "./runContext.js";
export type { RunContext } from "./runContext.js";
export type { SpendGuardAgentsOptions } from "./options.js";
export { deriveAgentSignature } from "./signature.js";
export { extractUsage } from "./usage.js";
export type { ExtractedUsage } from "./usage.js";
export {
  DecisionDenied,
  DecisionStopped,
  ApprovalRequired,
  SidecarUnavailable,
  SpendGuardError,
} from "./errors.js";
export { VERSION } from "./version.js";
