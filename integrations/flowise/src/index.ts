// @spendguard/flowise-nodes — public barrel.
//
// Flowise's loader discovers each node file under `dist/nodes/` and
// inspects its default export for `nodeClass`. The barrel re-exports the
// wrapper class for direct consumers (TS users embedding the wrapper
// outside Flowise) and the version helper.
//
// Locked at design.md §4: the only public class is
// `SpendGuardChatModelWrapper`. Anything more would expand the
// surface beyond the LOCKED design.

export { VERSION } from "./version.js";
export {
  SpendGuardChatModelWrapper,
  NODE_VERSION,
  type FlowiseNodeInput,
  type FlowiseNodeData,
  type FlowiseCommonObject,
  type MutableChatModel,
} from "./nodes/SpendGuardChatModelWrapper.js";
export {
  buildClaimEstimator,
  DEFAULT_CLAIM_ATOMIC,
  DEFAULT_CLAIM_SCOPE,
  type ClaimEntry,
  type ClaimEstimatorFn,
  type BuildClaimEstimatorArgs,
} from "./claimEstimator.js";
