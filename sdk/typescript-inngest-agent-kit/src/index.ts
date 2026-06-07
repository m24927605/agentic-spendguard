// @spendguard/inngest-agent-kit — Inngest AgentKit adapter for SpendGuard
// budget guardrails.
//
// SLICE 1 ships the package skeleton; SLICE 2 adds the `wrapWithSpendGuard`
// factory + locked options surface; SLICE 3 lands the reserve / commit /
// retry-dedup wiring. The SLICE 1+2+3 bundle is the Phase-2 final
// deliverable.
//
// Public surface (LOCKED per design.md §4 / review-standards.md §1):
//   - `wrapWithSpendGuard`                     — the factory.
//   - `WrapWithSpendGuardOptions`              — the locked options shape.
//   - `ClaimEstimatorInput`                    — estimator-input shape.
//   - `ClaimEstimator`                         — estimator function type.
//   - `CallSignatureFn`                        — optional signature override.
//   - `deriveIdentity` / `deriveStepIdempotencyKey`
//                                              — pure ID helpers, useful
//                                                for test-side parity probes
//                                                without spinning up a
//                                                client.
//   - `ApprovalRequired` / `DecisionDenied` /
//     `DecisionStopped` / `DecisionSkipped` /
//     `SidecarUnavailable` / `SpendGuardError` — re-exports from
//                                                @spendguard/sdk so
//                                                `err instanceof X` works
//                                                without the consumer
//                                                taking a second import.
//   - `VERSION`                                — package version constant.
//
// No `default` export — review-standards.md §1.6.
//
// Tree-shaking: every export above is a named export; tsup `splitting:
// false` + `treeshake: true` strip any unreferenced `@spendguard/sdk`
// symbol from the published bundle (review-standards.md §8.4).

export { wrapWithSpendGuard } from "./wrapWithSpendGuard.js";
export type {
  StepAi,
  InngestRuntimeCtx,
} from "./wrapWithSpendGuard.js";
export type {
  CallSignatureFn,
  ClaimEstimator,
  ClaimEstimatorInput,
  WrapWithSpendGuardOptions,
} from "./options.js";
export { deriveIdentity, deriveStepIdempotencyKey } from "./ids.js";
export type { DerivedIdentity } from "./ids.js";
export { extractProviderEventId, extractTotalTokens } from "./extract.js";
export {
  ApprovalRequired,
  DecisionDenied,
  DecisionSkipped,
  DecisionStopped,
  SidecarUnavailable,
  SpendGuardError,
} from "./errors.js";
export { VERSION } from "./version.js";
