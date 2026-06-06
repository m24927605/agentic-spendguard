// SpendGuard SDK — public barrel.
//
// Every symbol design.md §4.1 enumerates is re-exported here. Adapters
// (D04 / D06 / D08 / D29) `import { ... } from "@spendguard/sdk"`; subpath
// imports (`@spendguard/sdk/client`, `@spendguard/sdk/errors`, …) are honored
// for tree-shaking.
//
// **No `default export`** anywhere in this file — review-standards §1.7
// enforces named-export-only.
//
// SLICE 3 surfaces what's locked here:
//   - `SpendGuardClient` + the LOCKED config / outcome types (design §4.2).
//   - Full error hierarchy (design §4.5).
//   - Default deadlines + `VERSION` constant (design §3.2 / §4.2).
//   - Env-var helpers (`fromEnv` is a static method on the client).
//
// SLICE 6 (COV_S05_06) adds the three adapter-facing helper modules:
//   - `newUuid7` + `deriveIdempotencyKey` + `deriveUuidFromSignature` +
//     `workloadInstanceId` from `./ids.js`.
//   - `computePromptHash` from `./promptHash.js`.
//   - `PricingLookup` + `USD_MICROS_PER_USD` + `PriceKey` + `PriceTable`
//     from `./pricing.js`.
//
// **NOT re-exported here**:
//   - `DEMO_PRICING` lives on the `@spendguard/sdk/pricing/demo` subpath
//     only. The full snapshot is ~3 KB minified and re-exporting it from
//     the main barrel would unconditionally pull the embedded entries into
//     the main bundle even for adapters that never call demo helpers.
//
// What's INTENTIONALLY NOT exported yet (anti-scope per future slice docs):
//   - `withRunPlan` / `currentRunPlan` — SLICE 7.
// The placeholder re-exports for those land in their respective slices to
// avoid forward-shipping a half-implemented symbol that a downstream adapter
// could accidentally consume.

// ── Client ────────────────────────────────────────────────────────────────

export {
  SpendGuardClient,
  DEFAULT_CAPABILITY_LEVEL,
  DEFAULT_DECISION_TIMEOUT_MS,
  DEFAULT_HANDSHAKE_TIMEOUT_MS,
  DEFAULT_PROTOCOL_VERSION,
  DEFAULT_PUBLISH_TIMEOUT_MS,
  DEFAULT_TRACE_TIMEOUT_MS,
} from "./client.js";

export type {
  ApplyFailedRequest,
  BudgetClaim,
  ClaimEstimate,
  CommitEstimatedRequest,
  DecisionOutcome,
  EmitLlmCallPostRequest,
  HandshakeOutcome,
  PricingFreeze,
  PublishOutcomeRequest,
  QueryBudgetRequest,
  QueryBudgetResult,
  ReleaseOutcome,
  ReleaseRequest,
  ReserveRequest,
  ResumeAfterApprovalRequest,
  SpanRecord,
  UnitRef,
} from "./client.js";

// ── Config ────────────────────────────────────────────────────────────────

// `SpendGuardClientOptions` is the LOCKED §4.1 spec name; `SpendGuardClientConfig`
// is the slice-doc-internal shape (identical type via `type ... = ...` alias).
// Adapters in D04 / D06 / D08 / D29 import the spec name; the slice author's
// internal shape is exposed to keep refactors locally addressable.
export type {
  SpendGuardClientConfig,
  SpendGuardClientOptions,
  ResolvedConfig,
  RunProjectionPolicy,
} from "./config.js";
// `validateConfig` is intentionally NOT re-exported — it's a constructor-internal
// helper. Adapters do not need to revalidate.

// ── Env helpers ───────────────────────────────────────────────────────────

export { DEFAULT_SOCKET_PATH } from "./env.js";
export type { ResolvedEnvConfig } from "./env.js";

// ── Errors ────────────────────────────────────────────────────────────────

export {
  ApprovalBundleHotReloadedError,
  ApprovalDeniedError,
  ApprovalLapsedError,
  ApprovalRequired,
  DecisionDenied,
  DecisionSkipped,
  DecisionStopped,
  HandshakeError,
  MutationApplyFailed,
  SidecarUnavailable,
  SpendGuardConfigError,
  SpendGuardConnectionError,
  SpendGuardDecisionError,
  SpendGuardError,
} from "./errors.js";

export type {
  ApprovalRequiredInit,
  ApprovalResumeClient,
  DecisionDeniedInit,
} from "./errors.js";

// ── Version constant ──────────────────────────────────────────────────────

export { VERSION } from "./version.js";

// ── SLICE 6: ids / promptHash / pricing helpers ──────────────────────────

export {
  deriveIdempotencyKey,
  deriveUuidFromSignature,
  newUuid7,
  workloadInstanceId,
} from "./ids.js";

export { computePromptHash } from "./promptHash.js";

export { PricingLookup, USD_MICROS_PER_USD } from "./pricing.js";

export type { PriceKey, PriceTable } from "./pricing.js";
