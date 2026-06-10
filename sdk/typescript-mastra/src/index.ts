// src/index.ts — public barrel of @spendguard/mastra. Named exports only.
//
// design.md §5 verbatim barrel (COV_D38_02 completes the COV_D38_01
// placeholder subset). Exports EXACTLY the §5 symbols: no `default` export,
// no re-export of other `@spendguard/sdk` symbols (consumers import
// `ApprovalRequired`, `DecisionStopped`, `HandshakeError`, etc. from the
// substrate directly — D06 anti-list discipline; `DecisionStopped` /
// `ApprovalRequired` are subclasses of `DecisionDenied`, so
// `instanceof DecisionDenied` catches all denial flavours).

export { SpendGuardProcessor } from "./processor.js";
export type {
  SpendGuardProcessorOptions,
  ClaimEstimator,
  ClaimEstimatorInput,
} from "./options.js";
export { DecisionDenied, SidecarUnavailable, SpendGuardError } from "./errors.js";
export { VERSION } from "./version.js";
