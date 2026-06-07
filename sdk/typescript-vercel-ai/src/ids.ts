// `deriveIdempotencyKey` — adapter-side helper that maps Vercel AI SDK's
// `LanguageModelV1` call surface onto the substrate's canonical
// `(tenantId, sessionId, runId, stepId, llmCallId, trigger)` key shape.
//
// SLICE 3 wires only the `LLM_CALL_PRE` trigger — that is the single boundary
// where the adapter calls `client.reserve()` from `transformParams`. The
// mapping rule (LOCKED for SLICE 3):
//
//   - `sessionId`   := `parentRunId ?? runId`
//                      The Vercel AI SDK does not surface a parent-call id of
//                      its own — `transformParams` only sees the call's
//                      `params` reference. The optional `parentRunId`
//                      threaded through `SpendGuardMiddlewareOptions` /
//                      `currentRunPlan()` (deferred to a later slice) maps
//                      onto `sessionId`; when unset, the `runId` itself
//                      stands in so the substrate idempotency cache still
//                      sees a stable "this call is its own session" key.
//   - `runId`       := caller-supplied id (D06 SLICE 2/3 lifts this from the
//                      consumer through `runIdProvider`; until SLICE 3 wires
//                      a richer RunPlan integration, we accept it via the
//                      `runId` arg here and let the caller decide its source).
//   - `stepId`      := constant `"llm_call"` (SLICE 3 lock; design.md §6.3
//                      anticipates richer step-id shaping in later slices —
//                      mirrors D04 SLICE 3 lock exactly).
//   - `llmCallId`   := caller-supplied id (defaults to `runId` when the
//                      caller treats the call as the run boundary, exactly
//                      like D04 SLICE 3).
//   - `trigger`     := constant `"LLM_CALL_PRE"`
//
// Cross-language guarantee: for byte-identical
// `(tenantId, runId, parentRunId)` inputs, this helper returns the same
// `sg-…` key the D04 LangChain adapter would derive — both delegate to
// `@spendguard/sdk::deriveIdempotencyKey` with identical field
// canonicalisation. Verified end-to-end by `tests/middleware.test.ts`
// "idempotencyKey derivation deterministic" cases.

import { deriveIdempotencyKey as sdkDeriveKey } from "@spendguard/sdk";

/**
 * Derive the canonical idempotency key for a Vercel AI SDK LLM-call
 * boundary.
 *
 * The same `(tenantId, runId, parentRunId)` triple — invoked from any number
 * of retry attempts within a single AI SDK call (`maxRetries` default 2 in
 * v4+) — produces the same key, so the sidecar's idempotency cache and the
 * ledger's `UNIQUE` constraint collapse onto the first decision.
 *
 * @param opts.tenantId    - SpendGuard tenant the call is billed to.
 * @param opts.runId       - Caller-supplied run id (UUID or opaque string).
 * @param opts.parentRunId - Optional parent-run id; when omitted the runId
 *                           itself stands in as the session boundary.
 * @returns                  `"sg-"` + 32 hex chars (BLAKE2b-128 digest).
 */
export function deriveIdempotencyKey(opts: {
  tenantId: string;
  runId: string;
  parentRunId?: string;
}): string {
  return sdkDeriveKey({
    tenantId: opts.tenantId,
    sessionId: opts.parentRunId ?? opts.runId,
    runId: opts.runId,
    stepId: "llm_call",
    llmCallId: opts.runId,
    trigger: "LLM_CALL_PRE",
  });
}
